import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

type CommandCheck = {
	name: string;
	cmd: string[];
};

function run(command: CommandCheck) {
	const result = spawnSync(command.cmd[0], command.cmd.slice(1), {
		encoding: "utf8",
	});

	return {
		...result,
		stdout: result.stdout?.trim() ?? "",
		stderr: result.stderr?.trim() ?? "",
	};
}

function requireSuccess(command: CommandCheck, message: string) {
	const result = run(command);
	if (result.status !== 0) {
		const details = result.stderr || result.stdout || "no output";
		throw new Error(
			`${message}\nCommand: ${command.cmd.join(" ")}\nDetails: ${details}`,
		);
	}
	return result;
}

function runningSiloProcesses() {
	if (process.platform !== "darwin") {
		return [];
	}

	const result = run({
		name: "ps",
		cmd: ["ps", "-Ao", "pid=,command="],
	});
	if (result.status !== 0) {
		return [];
	}

	return result.stdout
		.split("\n")
		.map((line) => line.trim())
		.filter(
			(line) =>
				line.includes("/Silo.app/Contents/MacOS/Silo") &&
				!line.includes("playwright test"),
		);
}

function findTomlValue(contents: string, key: string): string | null {
	const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
	const match = contents.match(
		new RegExp(`^\\s*${escaped}\\s*=\\s*"([^"]*)"\\s*$`, "m"),
	);
	return match?.[1] ?? null;
}

function assertFile(pathname: string, message: string) {
	if (!existsSync(pathname)) {
		throw new Error(`${message}\nMissing path: ${pathname}`);
	}
}

const sourceStateDir =
	process.env.SILO_E2E_SOURCE_STATE_DIR ?? path.join(os.homedir(), ".silo");
const configPath = path.join(sourceStateDir, "config.toml");

assertFile(
	configPath,
	"Silo state was not found. Run the app normally first so live credentials and project settings are available.",
);

requireSuccess(
	{ name: "git", cmd: ["git", "--version"] },
	"`git` is required for live e2e runs.",
);
requireSuccess(
	{ name: "gh", cmd: ["gh", "--version"] },
	"`gh` is required for live e2e runs.",
);
requireSuccess(
	{ name: "gh auth", cmd: ["gh", "auth", "status"] },
	"`gh` is installed but not authenticated.",
);
requireSuccess(
	{ name: "gcloud", cmd: ["gcloud", "version"] },
	"`gcloud` is required for live e2e runs.",
);
requireSuccess(
	{ name: "cargo tauri", cmd: ["cargo", "tauri", "--version"] },
	"`cargo tauri` is required for live e2e runs.",
);

const runningSilo = runningSiloProcesses();
if (runningSilo.length > 0) {
	throw new Error(
		`Close running Silo.app instances before live e2e.\n${runningSilo.join("\n")}`,
	);
}

const activeAccount = requireSuccess(
	{
		name: "gcloud account",
		cmd: [
			"gcloud",
			"auth",
			"list",
			"--filter=status:ACTIVE",
			"--format=value(account)",
		],
	},
	"`gcloud` is installed but no active account is configured.",
).stdout;

const activeProject = requireSuccess(
	{
		name: "gcloud project",
		cmd: ["gcloud", "config", "get-value", "project"],
	},
	"`gcloud` is installed but no default project is configured.",
).stdout;

if (!activeAccount || activeAccount === "(unset)") {
	throw new Error("`gcloud` has no active account.");
}

if (!activeProject || activeProject === "(unset)") {
	throw new Error("`gcloud` has no active project.");
}

const configContents = readFileSync(configPath, "utf8");
const keyFile = findTomlValue(configContents, "service_account_key_file");
if (keyFile) {
	assertFile(
		keyFile,
		"Silo config references a Google Cloud service account key that no longer exists.",
	);
}

console.log("Live e2e preflight passed");
console.log(`source state: ${sourceStateDir}`);
console.log(`gcloud account: ${activeAccount}`);
console.log(`gcloud project: ${activeProject}`);
