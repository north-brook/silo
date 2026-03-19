import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import {
	cpSync,
	createWriteStream,
	existsSync,
	mkdirSync,
	readdirSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
	type Browser,
	type BrowserContext,
	test as base,
	expect,
	type Page,
} from "@playwright/test";

type SiloApp = {
	artifactsDir: string;
	browser: Browser;
	context: BrowserContext;
	page: Page;
	process: ChildProcess;
	stateDir: string;
};

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "../../..");
const defaultSourceStateDir = path.join(os.homedir(), ".silo");
const bunCommand =
	process.env.BUN_BIN ?? (process.platform === "win32" ? "bun.exe" : "bun");
const tauriNoopBeforeDevConfig = JSON.stringify({
	build: {
		beforeDevCommand: 'node -e "process.exit(0)"',
	},
});

async function waitFor<T>(
	callback: () => Promise<T | undefined>,
	{ timeoutMs, description }: { timeoutMs: number; description: string },
) {
	const deadline = Date.now() + timeoutMs;
	let lastError: unknown;

	while (Date.now() < deadline) {
		try {
			const result = await callback();
			if (result !== undefined) {
				return result;
			}
		} catch (error) {
			lastError = error;
		}

		await new Promise((resolve) => setTimeout(resolve, 500));
	}

	const details =
		lastError instanceof Error ? ` Last error: ${lastError.message}` : "";
	throw new Error(`Timed out waiting for ${description}.${details}`);
}

function ensureDirectory(pathname: string) {
	mkdirSync(pathname, { recursive: true });
}

type RunningSiloProcess = {
	command: string;
	pid: number;
};

function listRunningSiloApps(): RunningSiloProcess[] {
	if (process.platform !== "darwin") {
		return [];
	}

	const output = spawnSync("ps", ["-Ao", "pid=,command="], {
		encoding: "utf8",
	});
	if (output.status !== 0) {
		return [];
	}

	return output.stdout
		.split("\n")
		.map((line: string) => line.trim())
		.filter((line: string) => line.length > 0)
		.map((line: string) => {
			const firstSpace = line.indexOf(" ");
			if (firstSpace < 0) {
				return null;
			}

			const pid = Number.parseInt(line.slice(0, firstSpace).trim(), 10);
			const command = line.slice(firstSpace + 1).trim();
			if (!Number.isFinite(pid)) {
				return null;
			}

			return { pid, command };
		})
		.filter((entry): entry is RunningSiloProcess => entry !== null)
		.filter(
			(entry) =>
				entry.command.includes("/Silo.app/Contents/MacOS/Silo") &&
				!entry.command.includes("playwright test"),
		);
}

function assertNoRunningSiloApp() {
	const running = listRunningSiloApps();

	if (running.length > 0) {
		throw new Error(
			`Close running Silo.app instances before live e2e.\n${running
				.map(({ pid, command }) => `${pid} ${command}`)
				.join("\n")}`,
		);
	}
}

async function stopOwnedSiloApps(initialPids: Set<number>) {
	for (const processInfo of listRunningSiloApps()) {
		if (!initialPids.has(processInfo.pid)) {
			try {
				process.kill(processInfo.pid, "SIGTERM");
			} catch {}
		}
	}

	await new Promise((resolve) => setTimeout(resolve, 1_000));

	for (const processInfo of listRunningSiloApps()) {
		if (!initialPids.has(processInfo.pid)) {
			try {
				process.kill(processInfo.pid, "SIGKILL");
			} catch {}
		}
	}
}

function resolveArtifactsDir(workerIndex: number) {
	const timestamp = new Date().toISOString().replace(/:/g, "-");
	return path.join(
		repoRoot,
		"test-results",
		"e2e",
		`worker-${workerIndex}-${timestamp}`,
	);
}

function seedStateDir(targetStateDir: string) {
	const sourceStateDir =
		process.env.SILO_E2E_SOURCE_STATE_DIR ?? defaultSourceStateDir;
	const sourceConfigPath = path.join(sourceStateDir, "config.toml");

	if (!existsSync(sourceConfigPath)) {
		throw new Error(
			`Live source state is missing ${sourceConfigPath}. Run \`bun run e2e:preflight\` first.`,
		);
	}

	ensureDirectory(targetStateDir);
	cpSync(sourceConfigPath, path.join(targetStateDir, "config.toml"));

	for (const entry of readdirSync(sourceStateDir)) {
		if (entry.endsWith("-silo-workspaces.json")) {
			cpSync(
				path.join(sourceStateDir, entry),
				path.join(targetStateDir, entry),
			);
		}
	}
}

async function waitForCdpReady(port: number) {
	return waitFor(
		async () => {
			const response = await fetch(`http://127.0.0.1:${port}/json/version`);
			if (!response.ok) {
				return undefined;
			}
			return response;
		},
		{ timeoutMs: 120_000, description: `CEF CDP endpoint on port ${port}` },
	);
}

async function canReachUrl(url: string) {
	try {
		const response = await fetch(url);
		return response.ok;
	} catch {
		return false;
	}
}

async function ensureDevServer(artifactsDir: string) {
	if (await canReachUrl("http://localhost:3000")) {
		return null;
	}

	const stdoutPath = path.join(artifactsDir, "vite.stdout.log");
	const stderrPath = path.join(artifactsDir, "vite.stderr.log");
	const viteProcess = spawn(bunCommand, ["run", "dev"], {
		cwd: repoRoot,
		detached: process.platform !== "win32",
		env: process.env,
		stdio: ["ignore", "pipe", "pipe"],
	});

	viteProcess.stdout?.pipe(createWriteStream(stdoutPath));
	viteProcess.stderr?.pipe(createWriteStream(stderrPath));

	await waitFor(
		async () =>
			(await canReachUrl("http://localhost:3000")) ? true : undefined,
		{ timeoutMs: 60_000, description: "the Vite dev server on port 3000" },
	);

	return viteProcess;
}

async function resolveAppPage(browser: Browser) {
	return waitFor(
		async () => {
			for (const context of browser.contexts()) {
				for (const page of context.pages()) {
					if (
						page.url().startsWith("http://tauri.localhost/") ||
						page.url().startsWith("http://localhost:3000")
					) {
						return { context, page };
					}
				}
			}

			return undefined;
		},
		{ timeoutMs: 120_000, description: "the main Silo page" },
	);
}

async function stopProcess(child: ChildProcess) {
	if (child.killed || child.exitCode !== null) {
		return;
	}

	const pid = child.pid;
	if (!pid) {
		child.kill("SIGTERM");
		return;
	}

	if (process.platform === "win32") {
		child.kill("SIGTERM");
		return;
	}

	try {
		process.kill(-pid, "SIGTERM");
	} catch {
		child.kill("SIGTERM");
	}

	await new Promise((resolve) => setTimeout(resolve, 2_000));

	if (child.exitCode === null) {
		try {
			process.kill(-pid, "SIGKILL");
		} catch {
			child.kill("SIGKILL");
		}
	}
}

export const test = base.extend<{ appPage: Page }, { siloApp: SiloApp }>({
	siloApp: [
		async ({ playwright }, use, workerInfo) => {
			const artifactsDir = resolveArtifactsDir(workerInfo.workerIndex);
			const stateDir = path.join(artifactsDir, "state");
			const cdpPort = 9222 + workerInfo.workerIndex;
			ensureDirectory(artifactsDir);
			const initialSiloPids = new Set(
				listRunningSiloApps().map((processInfo) => processInfo.pid),
			);
			assertNoRunningSiloApp();
			seedStateDir(stateDir);
			const viteProcess = await ensureDevServer(artifactsDir);

			const stdoutPath = path.join(artifactsDir, "tauri.stdout.log");
			const stderrPath = path.join(artifactsDir, "tauri.stderr.log");
			const child = spawn(
				"cargo",
				["tauri", "dev", "-c", tauriNoopBeforeDevConfig],
				{
					cwd: repoRoot,
					detached: process.platform !== "win32",
					env: {
						...process.env,
						SILO_CEF_REMOTE_DEBUGGING_PORT: String(cdpPort),
						SILO_STATE_DIR: stateDir,
					},
					stdio: ["ignore", "pipe", "pipe"],
				},
			);

			child.stdout?.pipe(createWriteStream(stdoutPath));
			child.stderr?.pipe(createWriteStream(stderrPath));

			try {
				await waitForCdpReady(cdpPort);
				const browser = await playwright.chromium.connectOverCDP(
					`http://127.0.0.1:${cdpPort}`,
				);
				const { context, page } = await resolveAppPage(browser);
				await page.waitForLoadState("domcontentloaded");

				await use({
					artifactsDir,
					browser,
					context,
					page,
					process: child,
					stateDir,
				});

				await browser.close();
			} finally {
				await stopProcess(child);
				if (viteProcess) {
					await stopProcess(viteProcess);
				}
				await stopOwnedSiloApps(initialSiloPids);
			}
		},
		{ scope: "worker" },
	],
	appPage: async ({ siloApp }, use) => {
		await use(siloApp.page);
	},
});

export { expect };
