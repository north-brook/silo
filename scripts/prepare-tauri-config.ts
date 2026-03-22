import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const workspaceRoot = path.resolve(import.meta.dir, "..");
const version = process.env.SILO_RELEASE_VERSION?.trim();
const updaterPublicKey = normalizeUpdaterPublicKeyForConfig(
	process.env.SILO_UPDATER_PUBLIC_KEY,
);

if (!version) {
	throw new Error("SILO_RELEASE_VERSION must be set");
}

if (!updaterPublicKey) {
	throw new Error("SILO_UPDATER_PUBLIC_KEY must be set");
}

const basePath = path.resolve(
	workspaceRoot,
	process.env.SILO_TAURI_CONFIG_BASE ?? "src-tauri/tauri.prod.conf.json",
);
const outputPath = path.resolve(
	workspaceRoot,
	process.env.SILO_TAURI_CONFIG_OUTPUT ?? "src-tauri/tauri.release.conf.json",
);

const config = JSON.parse(await readFile(basePath, "utf8")) as Record<string, unknown>;
config.version = version;
const plugins = ensureRecord(config, "plugins");
const updater = ensureRecord(plugins, "updater");
updater.pubkey = updaterPublicKey;

await mkdir(path.dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(config, null, "\t")}\n`);

process.stdout.write(`${path.relative(workspaceRoot, outputPath)}\n`);

function normalizeUpdaterPublicKeyForConfig(value: string | undefined): string | null {
	const trimmed = value?.trim();
	if (!trimmed) {
		return null;
	}

	if (trimmed.includes("untrusted comment:")) {
		return encodeCanonicalUpdaterPublicKey(trimmed);
	}

	const decoded = Buffer.from(trimmed, "base64").toString("utf8").trim();
	if (decoded.includes("untrusted comment:")) {
		return encodeCanonicalUpdaterPublicKey(decoded);
	}

	throw new Error(
		"SILO_UPDATER_PUBLIC_KEY must be minisign public key text or a base64-wrapped key file",
	);
}

function encodeCanonicalUpdaterPublicKey(value: string): string {
	return Buffer.from(`${value}\n`, "utf8").toString("base64");
}

function ensureRecord(parent: Record<string, unknown>, key: string): Record<string, unknown> {
	const value = parent[key];
	if (value && typeof value === "object" && !Array.isArray(value)) {
		return value as Record<string, unknown>;
	}

	const next: Record<string, unknown> = {};
	parent[key] = next;
	return next;
}
