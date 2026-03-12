"use client";

import { invoke } from "@tauri-apps/api/core";
import {
	debug as pluginDebug,
	error as pluginError,
	info as pluginInfo,
	warn as pluginWarn,
} from "@tauri-apps/plugin-log";

type LogLevel = "debug" | "info" | "warn" | "error";
type ConsoleMethod = "log" | "debug" | "info" | "warn" | "error";

const PATCHED_CONSOLE_KEY = "__siloLoggingPatched";
const MAX_LOG_MESSAGE_LENGTH = 8_000;

let frontendLoggingInitialized = false;

const pluginWriters: Record<LogLevel, (message: string) => Promise<void>> = {
	debug: pluginDebug,
	info: pluginInfo,
	warn: pluginWarn,
	error: pluginError,
};

export function initializeFrontendLogging() {
	if (typeof window === "undefined" || frontendLoggingInitialized) return;
	frontendLoggingInitialized = true;

	patchConsole();
	window.addEventListener("error", handleWindowError);
	window.addEventListener("unhandledrejection", handleUnhandledRejection);
	void writeLog("info", "frontend logging initialized");
}

export async function invokeLogged<T>(
	command: string,
	args?: Record<string, unknown>,
): Promise<T> {
	const startedAt = performance.now();
	const argKeys = args ? Object.keys(args).sort() : [];
	const argsSummary =
		argKeys.length === 0 ? "args=[]" : `args=[${argKeys.join(",")}]`;

	await writeLog("debug", `invoke ${command} started ${argsSummary}`);

	try {
		const result = await invoke<T>(command, args);
		await writeLog(
			"info",
			`invoke ${command} succeeded duration_ms=${Math.round(performance.now() - startedAt)}`,
		);
		return result;
	} catch (error) {
		await writeLog(
			"error",
			`invoke ${command} failed duration_ms=${Math.round(performance.now() - startedAt)} error=${formatLogArg(error)}`,
		);
		throw normalizeError(error);
	}
}

function patchConsole() {
	const consoleWithMarker = console as Console & {
		[PATCHED_CONSOLE_KEY]?: boolean;
	};
	if (consoleWithMarker[PATCHED_CONSOLE_KEY]) return;
	consoleWithMarker[PATCHED_CONSOLE_KEY] = true;

	const methods: Array<[ConsoleMethod, LogLevel]> = [
		["log", "info"],
		["debug", "debug"],
		["info", "info"],
		["warn", "warn"],
		["error", "error"],
	];

	for (const [method, level] of methods) {
		const original = console[method].bind(console);
		console[method] = (...args: unknown[]) => {
			original(...args);
			void writeLog(level, `${method}: ${formatLogArgs(args)}`);
		};
	}
}

function handleWindowError(event: ErrorEvent) {
	const details = [
		event.message && `message=${event.message}`,
		event.filename && `file=${event.filename}`,
		typeof event.lineno === "number" && event.lineno > 0
			? `line=${event.lineno}`
			: undefined,
		typeof event.colno === "number" && event.colno > 0
			? `column=${event.colno}`
			: undefined,
		event.error ? `error=${formatLogArg(event.error)}` : undefined,
	]
		.filter(Boolean)
		.join(" ");

	void writeLog("error", `window error ${details}`.trim());
}

function handleUnhandledRejection(event: PromiseRejectionEvent) {
	void writeLog(
		"error",
		`unhandled rejection reason=${formatLogArg(event.reason)}`,
	);
}

async function writeLog(level: LogLevel, message: string) {
	try {
		await pluginWriters[level](truncate(message));
	} catch {
		// Ignore logging transport failures so application behavior is unchanged.
	}
}

function formatLogArgs(args: unknown[]): string {
	return args.map((arg) => formatLogArg(arg)).join(" ");
}

function formatLogArg(value: unknown): string {
	if (value instanceof Error) {
		const parts = [value.name, value.message, value.stack].filter(Boolean);
		return parts.join(": ");
	}

	if (typeof value === "string") return value;
	if (
		typeof value === "number" ||
		typeof value === "boolean" ||
		typeof value === "bigint" ||
		value === null ||
		value === undefined
	) {
		return String(value);
	}

	if (typeof value === "object") {
		return safeStringify(value);
	}

	return String(value);
}

function safeStringify(value: object): string {
	try {
		const seen = new WeakSet<object>();
		return truncate(
			JSON.stringify(value, (_key, currentValue: unknown) => {
			if (currentValue instanceof Error) {
				return {
					name: currentValue.name,
					message: currentValue.message,
					stack: currentValue.stack,
				};
			}

			if (typeof currentValue === "bigint") {
				return currentValue.toString();
			}

			if (typeof currentValue === "object" && currentValue !== null) {
				if (seen.has(currentValue)) return "[Circular]";
				seen.add(currentValue);
			}

			return currentValue;
			}) ?? String(value),
		);
	} catch {
		return "[Unserializable]";
	}
}

function normalizeError(error: unknown): Error {
	return error instanceof Error ? error : new Error(String(error));
}

function truncate(message: string): string {
	if (message.length <= MAX_LOG_MESSAGE_LENGTH) {
		return message;
	}

	return `${message.slice(0, MAX_LOG_MESSAGE_LENGTH)}…`;
}
