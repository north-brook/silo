"use client";

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
	debug as pluginDebug,
	error as pluginError,
	info as pluginInfo,
	warn as pluginWarn,
} from "@tauri-apps/plugin-log";

type LogLevel = "debug" | "info" | "warn" | "error";
type ConsoleMethod = "log" | "debug" | "info" | "warn" | "error";
type InvokeLogMode = "full" | "errors_only" | "state_changes_only";

type InvokeArgs = Record<string, unknown>;

interface InvokeOptions<T> {
	log?: InvokeLogMode;
	key?: string;
	stateChanged?: (previous: T | undefined, next: T) => boolean;
}

const PATCHED_CONSOLE_KEY = "__siloLoggingPatched";
const MAX_LOG_MESSAGE_LENGTH = 8_000;
const FRONTEND_BOOT_ID = createBootId();

let frontendLoggingInitialized = false;
let frontendLoggingSuppressed = false;
const lastInvokeResults = new Map<string, unknown>();

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
	window.addEventListener("beforeunload", () => {
		frontendLoggingSuppressed = true;
	});
	window.addEventListener("pagehide", (event) => {
		if (!event.persisted) {
			frontendLoggingSuppressed = true;
		}
	});
	window.addEventListener("pageshow", (event) => {
		frontendLoggingSuppressed = false;
		void writeLog(
			"info",
			`frontend pageshow boot_id=${FRONTEND_BOOT_ID} persisted=${event.persisted} href=${window.location.href}`,
		);
	});
	void writeLog(
		"info",
		`frontend logging initialized boot_id=${FRONTEND_BOOT_ID} ${formatIpcDiagnostics()}`,
	);
}

export function invoke<T>(command: string): Promise<T>;
export function invoke<T>(
	command: string,
	options: InvokeOptions<T>,
): Promise<T>;
export function invoke<T>(command: string, args: InvokeArgs): Promise<T>;
export function invoke<T>(
	command: string,
	args: InvokeArgs,
	options: InvokeOptions<T>,
): Promise<T>;
export async function invoke<T>(
	command: string,
	argsOrOptions?: InvokeArgs | InvokeOptions<T>,
	maybeOptions?: InvokeOptions<T>,
): Promise<T> {
	const startedAt = performance.now();
	const { args, options } = normalizeInvokeParams(argsOrOptions, maybeOptions);
	const argKeys = args ? Object.keys(args).sort() : [];
	const argsSummary =
		argKeys.length === 0 ? "args=[]" : `args=[${argKeys.join(",")}]`;
	const mode = options.log ?? "full";
	const dedupeKey = options.key ?? command;

	if (mode === "full") {
		await writeLog("debug", `invoke ${command} started ${argsSummary}`);
	}

	try {
		const result = await tauriInvoke<T>(command, args);
		await logInvokeSuccess(command, result, {
			mode,
			key: dedupeKey,
			stateChanged: options.stateChanged,
			durationMs: Math.round(performance.now() - startedAt),
		});
		return result;
	} catch (error) {
		await writeLog(
			"error",
			`invoke ${command} failed duration_ms=${Math.round(performance.now() - startedAt)} error=${formatLogArg(error)}`,
		);
		if (isFailedToFetchError(error)) {
			await writeLog(
				"error",
				`invoke ${command} failed-to-fetch diagnostics boot_id=${FRONTEND_BOOT_ID} ${formatIpcDiagnostics(command)}`,
			);
		}
		throw normalizeError(error);
	}
}

function normalizeInvokeParams<T>(
	argsOrOptions?: InvokeArgs | InvokeOptions<T>,
	maybeOptions?: InvokeOptions<T>,
): {
	args?: InvokeArgs;
	options: InvokeOptions<T>;
} {
	if (maybeOptions) {
		return {
			args: argsOrOptions as InvokeArgs,
			options: maybeOptions,
		};
	}

	if (isInvokeOptions(argsOrOptions)) {
		return { options: argsOrOptions };
	}

	return {
		args: argsOrOptions as InvokeArgs | undefined,
		options: {},
	};
}

function isInvokeOptions<T>(
	value: InvokeArgs | InvokeOptions<T> | undefined,
): value is InvokeOptions<T> {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		return false;
	}

	const keys = Object.keys(value);
	return (
		keys.length > 0 &&
		keys.every(
			(key) => key === "log" || key === "key" || key === "stateChanged",
		)
	);
}

async function logInvokeSuccess<T>(
	command: string,
	result: T,
	options: {
		mode: InvokeLogMode;
		key: string;
		stateChanged?: (previous: T | undefined, next: T) => boolean;
		durationMs: number;
	},
) {
	if (options.mode === "errors_only") {
		return;
	}

	if (options.mode === "state_changes_only") {
		const previous = lastInvokeResults.get(options.key) as T | undefined;
		const changed = options.stateChanged
			? options.stateChanged(previous, result)
			: inferState(previous) !== inferState(result);
		if (!changed) {
			return;
		}

		lastInvokeResults.set(options.key, result);
		await writeLog(
			"info",
			`invoke ${command} state duration_ms=${options.durationMs} value=${inferState(result)}`,
		);
		return;
	}

	await writeLog(
		"info",
		`invoke ${command} succeeded duration_ms=${options.durationMs}`,
	);
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
			if (shouldLogIpcDiagnostics(args)) {
				void writeLog(
					level,
					`${method}: ipc diagnostics boot_id=${FRONTEND_BOOT_ID} ${formatIpcDiagnostics()}`,
				);
			}
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
	if (frontendLoggingSuppressed) {
		return;
	}
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

function inferState(value: unknown): string {
	return formatLogArg(value);
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

function shouldLogIpcDiagnostics(args: unknown[]): boolean {
	const combined = formatLogArgs(args);
	return (
		combined.includes("IPC custom protocol failed") ||
		combined.includes("Failed to fetch") ||
		combined.includes("Couldn't find callback id")
	);
}

function isFailedToFetchError(error: unknown): boolean {
	return formatLogArg(error).includes("Failed to fetch");
}

function formatIpcDiagnostics(command?: string): string {
	if (typeof window === "undefined") {
		return "ipc_diagnostics=unavailable";
	}

	const tauriInternals = (window as Window & {
		__TAURI_INTERNALS__?: {
			convertFileSrc?: (path: string, protocol?: string) => string;
		};
		ipc?: {
			postMessage?: unknown;
		};
	}).__TAURI_INTERNALS__;
	const ipcBridge = (window as Window & {
		ipc?: {
			postMessage?: unknown;
		};
	}).ipc;

	let ipcUrl = "unavailable";
	if (command && tauriInternals?.convertFileSrc) {
		try {
			ipcUrl = tauriInternals.convertFileSrc(command, "ipc");
		} catch (error) {
			ipcUrl = `error:${formatLogArg(error)}`;
		}
	}

	return [
		`href=${window.location.href}`,
		`ready_state=${document.readyState}`,
		`visibility=${document.visibilityState}`,
		`has_tauri_internals=${Boolean(tauriInternals)}`,
		`has_convert_file_src=${typeof tauriInternals?.convertFileSrc === "function"}`,
		`window_ipc_type=${typeof ipcBridge}`,
		`window_ipc_post_message_type=${typeof ipcBridge?.postMessage}`,
		`ipc_url=${ipcUrl}`,
	].join(" ");
}

function createBootId(): string {
	if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
		return crypto.randomUUID();
	}

	return `boot-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function truncate(message: string): string {
	if (message.length <= MAX_LOG_MESSAGE_LENGTH) {
		return message;
	}

	return `${message.slice(0, MAX_LOG_MESSAGE_LENGTH)}…`;
}
