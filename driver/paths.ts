import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export const repoRoot = path.resolve(__dirname, "..");
export const defaultSourceStateDir = path.join(os.homedir(), ".silo");
export const driverRootDir = path.join(repoRoot, "test-results", "driver");
export const driverSessionDir = path.join(driverRootDir, "sessions");
export const bunCommand =
	process.env.BUN_BIN ?? (process.platform === "win32" ? "bun.exe" : "bun");
export const tauriNoopBeforeDevConfig = JSON.stringify({
	build: {
		beforeDevCommand: 'node -e "process.exit(0)"',
	},
});
