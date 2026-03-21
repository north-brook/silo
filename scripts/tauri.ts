import path from "node:path";

type BuildFlavor = "dev" | "prod";

const DEV_CONFIG_PATH = "src-tauri/tauri.conf.json";
const PROD_CONFIG_PATH = "src-tauri/tauri.prod.conf.json";

const args = process.argv.slice(2);
const requestedFlavor = args[0] === "prod" || args[0] === "dev" ? args.shift() : null;
const flavor = (requestedFlavor ?? process.env.SILO_BUILD_FLAVOR ?? "dev") as BuildFlavor;
const configPath =
	process.env.SILO_TAURI_CONFIG_PATH ??
		(flavor === "prod" ? PROD_CONFIG_PATH : DEV_CONFIG_PATH);
const tauriArgs =
	args.length > 0 && !args[0].startsWith("-")
		? [args[0], "--config", configPath, ...args.slice(1)]
		: args;

const subprocess = Bun.spawn(["cargo", "tauri", ...tauriArgs], {
	cwd: path.resolve(import.meta.dir, ".."),
	stdin: "inherit",
	stdout: "inherit",
	stderr: "inherit",
	env: {
		...process.env,
		SILO_BUILD_FLAVOR: flavor,
		VITE_SILO_APP_FLAVOR: flavor,
	},
});

const exitCode = await subprocess.exited;
process.exit(exitCode);
