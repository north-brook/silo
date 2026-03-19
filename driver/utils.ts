import { mkdirSync } from "node:fs";
import net from "node:net";

export function sleep(ms: number) {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function waitFor<T>(
	callback: () => Promise<T | undefined>,
	{ timeoutMs, description }: { timeoutMs: number; description: string },
) {
	const deadline = Date.now() + timeoutMs;
	let lastError: unknown;

	while (Date.now() < deadline) {
		try {
			const result = await callback();
			if (result !== undefined) {
				return result;
			}
		} catch (error) {
			lastError = error;
		}

		await sleep(500);
	}

	const details =
		lastError instanceof Error ? ` Last error: ${lastError.message}` : "";
	throw new Error(`Timed out waiting for ${description}.${details}`);
}

export function ensureDirectory(pathname: string) {
	mkdirSync(pathname, { recursive: true });
}

export function createSessionId() {
	const timestamp = new Date()
		.toISOString()
		.replace(/[-:]/g, "")
		.replace(/\..+$/, "")
		.toLowerCase();
	return `${timestamp}-${crypto.randomUUID().slice(0, 8)}`;
}

export async function canReachUrl(url: string) {
	try {
		const response = await fetch(url);
		return response.ok;
	} catch {
		return false;
	}
}

async function canBindPort(port: number) {
	return new Promise<boolean>((resolve) => {
		const server = net.createServer();
		server.unref();
		server.once("error", () => resolve(false));
		server.listen(port, "127.0.0.1", () => {
			server.close(() => resolve(true));
		});
	});
}

export async function findAvailablePort(startPort = 9222) {
	for (let port = startPort; port < startPort + 200; port += 1) {
		if (await canBindPort(port)) {
			return port;
		}
	}

	throw new Error(`Unable to find an available port starting at ${startPort}`);
}
