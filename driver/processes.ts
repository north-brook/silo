import { spawnSync } from "node:child_process";
import { sleep } from "./utils";

export type RunningSiloProcess = {
	command: string;
	pid: number;
};

export function listRunningSiloApps(): RunningSiloProcess[] {
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
		.map((line) => line.trim())
		.filter((line) => line.length > 0)
		.map((line) => {
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

export function assertNoRunningSiloApp() {
	const running = listRunningSiloApps();
	if (running.length > 0) {
		throw new Error(
			`Close running Silo.app instances before using the driver.\n${running
				.map(({ pid, command }) => `${pid} ${command}`)
				.join("\n")}`,
		);
	}
}

export function isPidRunning(pid: number | null | undefined) {
	if (!pid) {
		return false;
	}

	try {
		process.kill(pid, 0);
		return true;
	} catch {
		return false;
	}
}

export async function stopProcessByPid(
	pid: number | null | undefined,
	options: { gracefulWaitMs?: number } = {},
) {
	if (!pid || !isPidRunning(pid)) {
		return;
	}

	const gracefulWaitMs = options.gracefulWaitMs ?? 2_000;

	if (process.platform === "win32") {
		try {
			process.kill(pid, "SIGTERM");
		} catch {}
		return;
	}

	try {
		process.kill(-pid, "SIGTERM");
	} catch {
		try {
			process.kill(pid, "SIGTERM");
		} catch {}
	}

	await sleep(gracefulWaitMs);

	if (!isPidRunning(pid)) {
		return;
	}

	try {
		process.kill(-pid, "SIGKILL");
	} catch {
		try {
			process.kill(pid, "SIGKILL");
		} catch {}
	}
}

export async function stopOwnedSiloApps(initialPids: Set<number>) {
	for (const processInfo of listRunningSiloApps()) {
		if (!initialPids.has(processInfo.pid)) {
			try {
				process.kill(processInfo.pid, "SIGTERM");
			} catch {}
		}
	}

	await sleep(1_000);

	for (const processInfo of listRunningSiloApps()) {
		if (!initialPids.has(processInfo.pid)) {
			try {
				process.kill(processInfo.pid, "SIGKILL");
			} catch {}
		}
	}
}
