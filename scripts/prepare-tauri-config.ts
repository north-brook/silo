import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const workspaceRoot = path.resolve(import.meta.dir, "..");
const version = process.env.SILO_RELEASE_VERSION?.trim();

if (!version) {
	throw new Error("SILO_RELEASE_VERSION must be set");
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

await mkdir(path.dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(config, null, "\t")}\n`);

process.stdout.write(`${path.relative(workspaceRoot, outputPath)}\n`);
