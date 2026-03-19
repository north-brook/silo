import { spawn } from "node:child_process";
import {
	closeSync,
	cpSync,
	existsSync,
	openSync,
	readdirSync,
	writeFileSync,
} from "node:fs";
import path from "node:path";
import {
	type Browser,
	type BrowserContext,
	chromium,
	type Page,
} from "@playwright/test";
import {
	bunCommand,
	defaultSourceStateDir,
	driverRootDir,
	repoRoot,
	tauriNoopBeforeDevConfig,
	traceDirFor,
} from "./paths";
import { runLivePreflight } from "./preflight";
import {
	assertNoRunningSiloApp,
	listRunningSiloApps,
	stopOwnedSiloApps,
	stopProcessByPid,
} from "./processes";
import type {
	ConnectedDriverSession,
	DriverSessionRecord,
	LaunchedDriverSession,
	LaunchSessionOptions,
} from "./types";
import {
	canReachUrl,
	createSessionId,
	ensureDirectory,
	findAvailablePort,
	waitFor,
} from "./utils";

export class DriverLaunchError extends Error {
	session: DriverSessionRecord;

	constructor(message: string, session: DriverSessionRecord) {
		super(message);
		this.name = "DriverLaunchError";
		this.session = session;
	}
}

function writeTraceManifest(session: DriverSessionRecord) {
	writeFileSync(
		session.manifestPath,
		`${JSON.stringify(
			{
				appLogPath: session.appLogPath,
				artifactsDir: session.artifactsDir,
				cdpPort: session.cdpPort,
				cdpUrl: session.cdpUrl,
				createdAt: session.createdAt,
				driverLogPath: session.driverLogPath,
				sessionId: session.id,
				sourceStateDir: session.sourceStateDir,
				stateDir: session.stateDir,
				tauriPid: session.tauriPid,
				tauriStderrPath: session.tauriStderrPath,
				tauriStdoutPath: session.tauriStdoutPath,
			traceDir: session.traceDir,
			traceId: session.traceId,
			videoMetadataPath: session.videoMetadataPath,
			videoPath: session.videoPath,
			videoRecorderPid: session.videoRecorderPid,
			videoStderrPath: session.videoStderrPath,
			videoStdoutPath: session.videoStdoutPath,
			vitePid: session.vitePid,
			viteStderrPath: session.viteStderrPath,
			viteStdoutPath: session.viteStdoutPath,
			},
			null,
			2,
		)}\n`,
	);
}

function seedStateDir(targetStateDir: string, sourceStateDir: string) {
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

async function ensureDevServer(artifactsDir: string) {
	if (await canReachUrl("http://localhost:3000")) {
		return null;
	}

	const stdoutPath = path.join(artifactsDir, "vite.stdout.log");
	const stderrPath = path.join(artifactsDir, "vite.stderr.log");
	const stdoutFd = openSync(stdoutPath, "a");
	const stderrFd = openSync(stderrPath, "a");
	const viteProcess = spawn(bunCommand, ["run", "dev"], {
		cwd: repoRoot,
		detached: process.platform !== "win32",
		env: process.env,
		stdio: ["ignore", stdoutFd, stderrFd],
	});
	closeSync(stdoutFd);
	closeSync(stderrFd);
	viteProcess.unref();

	await waitFor(
		async () =>
			(await canReachUrl("http://localhost:3000")) ? true : undefined,
		{ timeoutMs: 60_000, description: "the Vite dev server on port 3000" },
	);

	return viteProcess;
}

async function waitForCdpReady(cdpUrl: string) {
	return waitFor(
		async () => {
			const response = await fetch(`${cdpUrl}/json/version`);
			if (!response.ok) {
				return undefined;
			}
			return response;
		},
		{ timeoutMs: 120_000, description: `CEF CDP endpoint at ${cdpUrl}` },
	);
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

async function connectWithCdp(session: DriverSessionRecord) {
	const browser = await chromium.connectOverCDP(session.cdpUrl, {
		timeout: 10_000,
	});
	const { context, page } = await resolveAppPage(browser);
	await page.waitForLoadState("domcontentloaded");
	return { browser, context, page };
}

function spawnVideoRecorder(session: DriverSessionRecord) {
	const stdoutFd = openSync(session.videoStdoutPath, "a");
	const stderrFd = openSync(session.videoStderrPath, "a");
	const recorder = spawn(
		bunCommand,
		[
			"x",
			"tsx",
			"driver/recorder.ts",
			"--cdp-url",
			session.cdpUrl,
			"--output",
			session.videoPath,
			"--frames-dir",
			path.join(session.traceDir, "video-frames"),
			"--metadata-path",
			session.videoMetadataPath,
			"--fps",
			"12",
		],
		{
			cwd: repoRoot,
			detached: process.platform !== "win32",
			env: process.env,
			stdio: ["ignore", stdoutFd, stderrFd],
		},
	);
	closeSync(stdoutFd);
	closeSync(stderrFd);
	recorder.unref();
	return recorder.pid ?? null;
}

export async function launchDriverSession(
	options: LaunchSessionOptions = {},
): Promise<LaunchedDriverSession> {
	const sourceStateDir = options.sourceStateDir ?? defaultSourceStateDir;

	if (!options.skipPreflight) {
		runLivePreflight({ checkNoRunningSiloApp: true, sourceStateDir });
	} else {
		assertNoRunningSiloApp();
	}

	const id = options.id ?? createSessionId();
	const artifactsDir = options.artifactsDir ?? path.join(driverRootDir, id);
	const stateDir = path.join(artifactsDir, "state");
	const traceId = id;
	const traceDir = traceDirFor(sourceStateDir, traceId);
	const cdpPort = options.cdpPort ?? (await findAvailablePort(9222));
	const cdpUrl = `http://127.0.0.1:${cdpPort}`;

	ensureDirectory(artifactsDir);
	ensureDirectory(traceDir);

	const initialSiloPids = listRunningSiloApps().map(
		(processInfo) => processInfo.pid,
	);
	seedStateDir(stateDir, sourceStateDir);
	const viteProcess = await ensureDevServer(traceDir);

	const tauriStdoutPath = path.join(traceDir, "tauri.stdout.log");
	const tauriStderrPath = path.join(traceDir, "tauri.stderr.log");
	const viteStdoutPath = viteProcess
		? path.join(traceDir, "vite.stdout.log")
		: null;
	const viteStderrPath = viteProcess
		? path.join(traceDir, "vite.stderr.log")
		: null;
	const manifestPath = path.join(traceDir, "manifest.json");
	const driverLogPath = path.join(traceDir, "driver.jsonl");
	const appLogPath = path.join(traceDir, "app.log");
	const videoPath = path.join(traceDir, "video.mp4");
	const videoMetadataPath = path.join(traceDir, "video-metadata.json");
	const videoStdoutPath = path.join(traceDir, "video.stdout.log");
	const videoStderrPath = path.join(traceDir, "video.stderr.log");
	const tauriStdoutFd = openSync(tauriStdoutPath, "a");
	const tauriStderrFd = openSync(tauriStderrPath, "a");
	const tauriProcess = spawn(
		"cargo",
		["tauri", "dev", "-c", tauriNoopBeforeDevConfig],
		{
			cwd: repoRoot,
			detached: process.platform !== "win32",
			env: {
				...process.env,
				SILO_CEF_REMOTE_DEBUGGING_PORT: String(cdpPort),
				SILO_STATE_DIR: stateDir,
				SILO_TRACE_DIR: traceDir,
				SILO_TRACE_ID: traceId,
			},
			stdio: ["ignore", tauriStdoutFd, tauriStderrFd],
		},
	);
	closeSync(tauriStdoutFd);
	closeSync(tauriStderrFd);
	tauriProcess.unref();

	const session: DriverSessionRecord = {
		id,
		createdAt: new Date().toISOString(),
		artifactsDir,
		appLogPath,
		cdpPort,
		cdpUrl,
		driverLogPath,
		initialSiloPids,
		manifestPath,
		platform: process.platform,
		sourceStateDir,
		stateDir,
		tauriPid: tauriProcess.pid ?? 0,
		tauriStderrPath,
		tauriStdoutPath,
		traceDir,
		traceId,
		videoMetadataPath,
		videoPath,
		videoRecorderPid: null,
		videoStderrPath,
		videoStdoutPath,
		vitePid: viteProcess?.pid ?? null,
		viteStderrPath,
		viteStdoutPath,
	};
	writeTraceManifest(session);

	try {
		await waitForCdpReady(cdpUrl);
		const { browser, context, page } = await connectWithCdp(session);
		session.videoRecorderPid = spawnVideoRecorder(session);
		writeTraceManifest(session);
		return { session, browser, context, page };
	} catch (error) {
		await stopLaunchedSession(session);
		throw new DriverLaunchError(
			error instanceof Error ? error.message : String(error),
			session,
		);
	}
}

export async function connectToDriverSession(
	session: DriverSessionRecord,
): Promise<ConnectedDriverSession> {
	const { browser, context, page } = await connectWithCdp(session);
	return { session, browser, context, page };
}

export async function disconnectFromDriverSession(connection: {
	browser: Browser;
}) {
	await connection.browser.close();
}

export async function stopLaunchedSession(session: DriverSessionRecord) {
	await stopProcessByPid(session.videoRecorderPid, { gracefulWaitMs: 15_000 });
	await stopProcessByPid(session.tauriPid);
	if (session.vitePid) {
		await stopProcessByPid(session.vitePid);
	}
	await stopOwnedSiloApps(new Set(session.initialSiloPids));
}

export function resolveTitle(page: Page) {
	return page.title().catch(() => "");
}

export function collectOpenPages(context: BrowserContext) {
	return context.pages();
}
