import { mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { chromium } from "@playwright/test";

type Args = {
	cdpUrl: string;
	output: string;
	fps: number;
	framesDir: string;
	metadataPath: string;
};

function parseArgs(argv: string[]): Args {
	const flags = new Map<string, string>();
	for (let index = 0; index < argv.length; index += 1) {
		const token = argv[index];
		if (!token.startsWith("--")) {
			continue;
		}

		const key = token.slice(2);
		const next = argv[index + 1];
		if (!next || next.startsWith("--")) {
			throw new Error(`Missing value for --${key}`);
		}
		flags.set(key, next);
		index += 1;
	}

	const cdpUrl = flags.get("cdp-url");
	const output = flags.get("output");
	const framesDir = flags.get("frames-dir");
	const metadataPath = flags.get("metadata-path");
	const fps = Number.parseInt(flags.get("fps") ?? "12", 10);

	if (!cdpUrl || !output || !framesDir || !metadataPath) {
		throw new Error(
			"Usage: tsx driver/recorder.ts --cdp-url <url> --output <video.mp4> --frames-dir <dir> --metadata-path <file> [--fps 12]",
		);
	}
	if (!Number.isFinite(fps) || fps <= 0) {
		throw new Error(`Invalid --fps value: ${flags.get("fps") ?? ""}`);
	}

	return { cdpUrl, output, fps, framesDir, metadataPath };
}

function writeMetadata(
	metadataPath: string,
	value: Record<string, unknown>,
) {
	writeFileSync(metadataPath, `${JSON.stringify(value, null, 2)}\n`);
}

async function resolvePage(cdpUrl: string) {
	const browser = await chromium.connectOverCDP(cdpUrl, { timeout: 10_000 });
	for (const context of browser.contexts()) {
		for (const page of context.pages()) {
			if (
				page.url().startsWith("http://tauri.localhost/")
				|| page.url().startsWith("http://localhost:3000")
			) {
				await page.waitForLoadState("domcontentloaded").catch(() => undefined);
				return { browser, context, page };
			}
		}
	}

	await browser.close();
	throw new Error("Unable to resolve the main Silo page for recording.");
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const startedAt = new Date().toISOString();
	mkdirSync(args.framesDir, { recursive: true });
	writeMetadata(args.metadataPath, {
		encoded: false,
		fps: args.fps,
		frameCount: 0,
		output: args.output,
		startedAt,
		state: "starting",
	});

	const { browser, context, page } = await resolvePage(args.cdpUrl);
	const client = await context.newCDPSession(page);
	let frameCount = 0;
	let stopping = false;
	let stopPromise: Promise<void> | null = null;

	client.on("Page.screencastFrame", (payload) => {
		void (async () => {
			const framePath = path.join(
				args.framesDir,
				`frame-${String(frameCount).padStart(6, "0")}.jpg`,
			);
			frameCount += 1;
			writeFileSync(framePath, Buffer.from(payload.data, "base64"));
			writeMetadata(args.metadataPath, {
				encoded: false,
				fps: args.fps,
				frameCount,
				lastFrameMetadata: payload.metadata,
				output: args.output,
				startedAt,
				state: stopping ? "stopping" : "recording",
			});
			await client.send("Page.screencastFrameAck", {
				sessionId: payload.sessionId,
			});
		})();
	});

	await client.send("Page.enable");
	await client.send("Page.startScreencast", {
		everyNthFrame: 1,
		format: "jpeg",
		quality: 80,
	});
	writeMetadata(args.metadataPath, {
		encoded: false,
		fps: args.fps,
		frameCount,
		output: args.output,
		startedAt,
		state: "recording",
	});

	async function finalize() {
		if (stopPromise) {
			return stopPromise;
		}

		stopping = true;
		stopPromise = (async () => {
			try {
				await client.send("Page.stopScreencast").catch(() => undefined);
				await browser.close().catch(() => undefined);

				if (frameCount === 0) {
					writeMetadata(args.metadataPath, {
						encoded: false,
						fps: args.fps,
						frameCount,
						output: args.output,
						startedAt,
						state: "empty",
					});
					return;
				}

				const result = spawnSync(
					"ffmpeg",
					[
						"-y",
						"-framerate",
						String(args.fps),
						"-i",
						path.join(args.framesDir, "frame-%06d.jpg"),
						"-c:v",
						"libx264",
						"-pix_fmt",
						"yuv420p",
						"-movflags",
						"+faststart",
						args.output,
					],
					{ encoding: "utf8" },
				);

				if (result.status !== 0) {
					writeMetadata(args.metadataPath, {
						encoded: false,
						error: result.stderr || result.stdout || "ffmpeg failed",
						fps: args.fps,
						frameCount,
						output: args.output,
						startedAt,
						state: "encode_failed",
					});
					process.exitCode = result.status ?? 1;
					return;
				}

				rmSync(args.framesDir, { force: true, recursive: true });
				writeMetadata(args.metadataPath, {
					encoded: true,
					fps: args.fps,
					frameCount,
					output: args.output,
					sizeBytes: statSync(args.output).size,
					startedAt,
					state: "completed",
				});
			} catch (error) {
				writeMetadata(args.metadataPath, {
					encoded: false,
					error: error instanceof Error ? error.message : String(error),
					fps: args.fps,
					frameCount,
					output: args.output,
					startedAt,
					state: "failed",
				});
				process.exitCode = 1;
			}
		})();

		return stopPromise;
	}

	process.on("SIGINT", () => {
		void finalize().finally(() => process.exit());
	});
	process.on("SIGTERM", () => {
		void finalize().finally(() => process.exit());
	});
	process.on("beforeExit", () => {
		void finalize();
	});
	process.on("uncaughtException", (error) => {
		writeMetadata(args.metadataPath, {
			encoded: false,
			error: error instanceof Error ? error.message : String(error),
			fps: args.fps,
			frameCount,
			output: args.output,
			startedAt,
			state: "failed",
		});
		process.exit(1);
	});
}

void main().catch((error) => {
	console.error(error instanceof Error ? error.message : String(error));
	process.exit(1);
});
