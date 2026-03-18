import react from "@vitejs/plugin-react";
import path from "node:path";
import { defineConfig } from "vite";

const devHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
	plugins: [react()],
	resolve: {
		alias: {
			"@": path.resolve(__dirname, "src"),
		},
	},
	clearScreen: false,
	server: {
		host: devHost || "localhost",
		port: 3000,
		strictPort: true,
		hmr: {
			protocol: "ws",
			host: devHost || "localhost",
			port: 3000,
		},
	},
	envPrefix: ["VITE_", "TAURI_ENV_*"],
	build: {
		target:
			process.env.TAURI_ENV_PLATFORM === "windows"
				? ["chrome105"]
				: ["safari13"],
		minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
		sourcemap: !!process.env.TAURI_ENV_DEBUG,
		rollupOptions: {
			input: {
				main: "index.html",
				workspace: "workspace/index.html",
				workspaceSession: "workspace/session/index.html",
				workspaceSaving: "workspace/saving/index.html",
				workspaceResuming: "workspace/resuming/index.html",
			},
		},
	},
});
