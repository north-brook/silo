#!/usr/bin/env bun

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

type TomlRecord = Record<string, unknown>;
type GcloudTarget = [account: string, project: string];
type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): TomlRecord {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		return {};
	}

	return value as TomlRecord;
}

function asString(value: unknown): string {
	return typeof value === "string" ? value.trim() : "";
}

function resolvedAccount(
	globalGcloud: TomlRecord,
	projectGcloud: TomlRecord,
): string {
	const serviceAccount = asString(globalGcloud.service_account);
	if (serviceAccount) {
		return serviceAccount;
	}

	const override = asString(projectGcloud.account);
	if (override) {
		return override;
	}

	return asString(globalGcloud.account);
}

function resolvedProject(
	globalGcloud: TomlRecord,
	projectGcloud: TomlRecord,
): string {
	const override = asString(projectGcloud.project);
	if (override) {
		return override;
	}

	return asString(globalGcloud.project);
}

async function loadGcloudTargets(configPath: string): Promise<GcloudTarget[]> {
	const config = asRecord(Bun.TOML.parse(await Bun.file(configPath).text()));
	const globalGcloud = asRecord(config.gcloud);
	const projects = asRecord(config.projects);

	const targets: GcloudTarget[] = [];
	const seen = new Set<string>();

	for (const projectConfig of Object.values(projects)) {
		const projectGcloud = asRecord(asRecord(projectConfig).gcloud);
		const account = resolvedAccount(globalGcloud, projectGcloud);
		const project = resolvedProject(globalGcloud, projectGcloud);
		if (!account || !project) {
			continue;
		}

		const key = `${account}\u0000${project}`;
		if (!seen.has(key)) {
			seen.add(key);
			targets.push([account, project]);
		}
	}

	const globalAccount =
		asString(globalGcloud.service_account) || asString(globalGcloud.account);
	const globalProject = asString(globalGcloud.project);
	if (globalAccount && globalProject) {
		const key = `${globalAccount}\u0000${globalProject}`;
		if (!seen.has(key)) {
			targets.push([globalAccount, globalProject]);
		}
	}

	return targets;
}

function runGcloudJson(
	account: string,
	project: string,
	args: string[],
): JsonRecord[] {
	const result = spawnSync(
		"gcloud",
		[`--account=${account}`, `--project=${project}`, ...args, "--format=json"],
		{
			encoding: "utf8",
		},
	);

	if (result.status !== 0) {
		const message =
			result.stderr.trim() ||
			result.error?.message ||
			`gcloud exited with status ${result.status ?? "unknown"}`;
		throw new Error(message);
	}

	const payload = result.stdout.trim();
	if (!payload) {
		return [];
	}

	const value = JSON.parse(payload) as unknown;
	if (Array.isArray(value)) {
		return value.filter(
			(entry): entry is JsonRecord =>
				Boolean(entry) && typeof entry === "object" && !Array.isArray(entry),
		);
	}

	if (value && typeof value === "object") {
		return [value as JsonRecord];
	}

	return [];
}

function runGcloud(account: string, project: string, args: string[]): void {
	const result = spawnSync(
		"gcloud",
		[`--account=${account}`, `--project=${project}`, ...args],
		{
			stdio: "inherit",
		},
	);

	if (result.status !== 0) {
		const message =
			result.error?.message ||
			`gcloud exited with status ${result.status ?? "unknown"}`;
		throw new Error(message);
	}
}

function listSiloInstances(account: string, project: string): GcloudTarget[] {
	const instances = runGcloudJson(account, project, [
		"compute",
		"instances",
		"list",
		"--filter=name~'.*-silo-.*'",
	]);

	const results: GcloudTarget[] = [];
	for (const instance of instances) {
		const name = instance.name;
		const zone = instance.zone;
		if (typeof name !== "string" || typeof zone !== "string") {
			continue;
		}

		const zoneParts = zone.split("/");
		const zoneName = zoneParts[zoneParts.length - 1]?.trim() ?? "";
		if (zoneName) {
			results.push([name, zoneName]);
		}
	}

	return results;
}

function listTemplateSnapshots(account: string, project: string): string[] {
	const snapshots = runGcloudJson(account, project, [
		"compute",
		"snapshots",
		"list",
		"--filter=labels.template=true",
	]);

	return snapshots
		.map((snapshot) => snapshot.name)
		.filter(
			(name): name is string => typeof name === "string" && name.length > 0,
		);
}

async function main(): Promise<number> {
	const dryRun = process.argv.slice(2).includes("--dry-run");
	const configPath = join(homedir(), ".silo", "config.toml");

	if (!existsSync(configPath)) {
		console.error(`config not found: ${configPath}`);
		return 1;
	}

	const targets = await loadGcloudTargets(configPath);
	if (targets.length === 0) {
		console.log("no configured gcloud targets found");
		return 0;
	}

	for (const [account, project] of targets) {
		console.log(`[${project}] account=${account}`);

		const instances = listSiloInstances(account, project);
		if (instances.length > 0) {
			for (const [name, zone] of instances) {
				console.log(`delete instance ${name} (${zone})`);
				if (!dryRun) {
					runGcloud(account, project, [
						"compute",
						"instances",
						"delete",
						name,
						`--zone=${zone}`,
						"--quiet",
					]);
				}
			}
		} else {
			console.log("no silo instances found");
		}

		const snapshots = listTemplateSnapshots(account, project);
		if (snapshots.length > 0) {
			for (const name of snapshots) {
				console.log(`delete snapshot ${name}`);
				if (!dryRun) {
					runGcloud(account, project, [
						"compute",
						"snapshots",
						"delete",
						name,
						"--quiet",
					]);
				}
			}
		} else {
			console.log("no template snapshots found");
		}
	}

	return 0;
}

const exitCode = await main();
process.exit(exitCode);
