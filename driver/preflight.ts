import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { defaultSourceStateDir } from "./paths";
import { assertNoRunningSiloApp } from "./processes";

type CommandCheck = {
	cmd: string[];
};

export type LivePreflightOptions = {
	checkNoRunningSiloApp?: boolean;
	sourceStateDir?: string;
};

export type LivePreflightResult = {
	activeAccount: string;
	activeProject: string;
	sourceStateDir: string;
};

function run(command: CommandCheck) {
	const result = spawnSync(command.cmd[0], command.cmd.slice(1), {
		encoding: "utf8",
	});

	return {
		...result,
		stderr: result.stderr?.trim() ?? "",
		stdout: result.stdout?.trim() ?? "",
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

function findTomlValue(contents: string, key: string) {
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

export function runLivePreflight(
	options: LivePreflightOptions = {},
): LivePreflightResult {
	const sourceStateDir = options.sourceStateDir ?? defaultSourceStateDir;
	const configPath = path.join(sourceStateDir, "config.toml");

	assertFile(
		configPath,
		"Silo state was not found. Run the app normally first so live credentials and project settings are available.",
	);

	requireSuccess({ cmd: ["git", "--version"] }, "`git` is required.");
	requireSuccess({ cmd: ["gh", "--version"] }, "`gh` is required.");
	requireSuccess(
		{ cmd: ["gh", "auth", "status"] },
		"`gh` is installed but not authenticated.",
	);
	requireSuccess({ cmd: ["gcloud", "version"] }, "`gcloud` is required.");
	requireSuccess(
		{ cmd: ["cargo", "tauri", "--version"] },
		"`cargo tauri` is required.",
	);

	if (options.checkNoRunningSiloApp !== false) {
		assertNoRunningSiloApp();
	}

	const activeAccount = requireSuccess(
		{
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

	return {
		activeAccount,
		activeProject,
		sourceStateDir,
	};
}
