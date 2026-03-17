import { PHASE_DEVELOPMENT_SERVER } from "next/constants.js";

export default function nextConfig(phase) {
	const isDev = phase === PHASE_DEVELOPMENT_SERVER;

	/** @type {import('next').NextConfig} */
	return {
		output: "export",
		trailingSlash: true,
		reactStrictMode: process.env.NODE_ENV === "production",
		images: {
			unoptimized: true,
		},
		devIndicators: false,
		// Keep scripts/styles same-origin with tauri.localhost in dev. HMR websocket
		// is redirected separately in the client because Tauri's dev proxy does not
		// forward websocket upgrades under CEF yet.
		assetPrefix: undefined,
	};
}
