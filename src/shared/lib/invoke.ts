import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
	debug as pluginDebug,
	error as pluginError,
	info as pluginInfo,
	warn as pluginWarn,
} from "@tauri-apps/plugin-log";
import { domFocusSnapshot } from "./focus-debug";
import { InvokeResultCache } from "./invoke-cache";

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
const MAX_RETAINED_INVOKE_RESULTS = 96;
const INVOKE_RESULT_TTL_MS = 10 * 60 * 1000;

let frontendLoggingInitialized = false;
let frontendLoggingSuppressed = false;
const lastInvokeResults = new InvokeResultCache<unknown>({
	maxEntries: MAX_RETAINED_INVOKE_RESULTS,
	ttlMs: INVOKE_RESULT_TTL_MS,
});
let webSocketPatched = false;

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
	patchWebSocket();
	window.addEventListener("error", handleWindowError);
	window.addEventListener("unhandledrejection", handleUnhandledRejection);
	window.addEventListener("focus", () => {
		void writeLog(
			"info",
			`frontend window focus boot_id=${FRONTEND_BOOT_ID} href=${window.location.href} ${formatDomFocusDiagnostics()}`,
		);
	});
	window.addEventListener("blur", () => {
		void writeLog(
			"info",
			`frontend window blur boot_id=${FRONTEND_BOOT_ID} href=${window.location.href} ${formatDomFocusDiagnostics()}`,
		);
	});
	document.addEventListener("visibilitychange", () => {
		void writeLog(
			"info",
			`frontend visibilitychange boot_id=${FRONTEND_BOOT_ID} visibility=${document.visibilityState} href=${window.location.href} ${formatDomFocusDiagnostics()}`,
		);
	});
	window.addEventListener("popstate", () => {
		void writeLog(
			"info",
			`frontend popstate boot_id=${FRONTEND_BOOT_ID} href=${window.location.href}`,
		);
	});
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
		`frontend logging initialized boot_id=${FRONTEND_BOOT_ID} ${formatIpcDiagnostics()} ${formatNavigationDiagnostics()}`,
	);
}

export function hotPollLogMode() {
	return import.meta.env.DEV ? "state_changes_only" : "errors_only";
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

function formatDomFocusDiagnostics() {
	const snapshot = domFocusSnapshot();
	return `document_has_focus=${snapshot.documentHasFocus} active_element=${snapshot.activeElement}`;
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

function patchWebSocket() {
	if (typeof window === "undefined" || webSocketPatched) return;
	webSocketPatched = true;

	const NativeWebSocket = window.WebSocket;
	const hmrOverrideOrigin =
		import.meta.env.VITE_TAURI_HMR_ORIGIN ?? "ws://localhost:3000";
	class InstrumentedWebSocket extends NativeWebSocket {
		constructor(url: string | URL, protocols?: string | string[]) {
			const normalizedUrl = typeof url === "string" ? url : url.toString();
			const rewrittenUrl = rewriteHmrWebSocketUrl(
				normalizedUrl,
				hmrOverrideOrigin,
			);
			super(rewrittenUrl, protocols);
			const shouldTrace =
				normalizedUrl.includes("tauri.localhost") ||
				normalizedUrl.includes("localhost:3000");
			if (!shouldTrace) {
				return;
			}

			void writeLog(
				"info",
				`websocket create boot_id=${FRONTEND_BOOT_ID} url=${normalizedUrl} rewritten_url=${rewrittenUrl} ready_state=${this.readyState}`,
			);
			this.addEventListener("open", () => {
				void writeLog(
					"info",
					`websocket open boot_id=${FRONTEND_BOOT_ID} url=${normalizedUrl} rewritten_url=${rewrittenUrl} protocol=${this.protocol}`,
				);
			});
			this.addEventListener("error", () => {
				void writeLog(
					"warn",
					`websocket error boot_id=${FRONTEND_BOOT_ID} url=${normalizedUrl} rewritten_url=${rewrittenUrl} ready_state=${this.readyState}`,
				);
			});
			this.addEventListener("close", (event) => {
				void writeLog(
					"warn",
					`websocket close boot_id=${FRONTEND_BOOT_ID} url=${normalizedUrl} rewritten_url=${rewrittenUrl} code=${event.code} reason=${event.reason || "none"} clean=${event.wasClean}`,
				);
			});
		}
	}

	window.WebSocket = InstrumentedWebSocket;
}

function rewriteHmrWebSocketUrl(url: string, overrideOrigin: string) {
	try {
		const parsedUrl = new URL(url);
		if (parsedUrl.hostname !== "tauri.localhost") {
			return url;
		}

		const parsedOverride = new URL(overrideOrigin);
		parsedUrl.protocol = parsedOverride.protocol;
		parsedUrl.hostname = parsedOverride.hostname;
		parsedUrl.port = parsedOverride.port;
		return parsedUrl.toString();
	} catch {
		return url;
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

function formatNavigationDiagnostics() {
	const navigationEntries = performance.getEntriesByType("navigation");
	const navigationEntry = navigationEntries[navigationEntries.length - 1] as
		| PerformanceNavigationTiming
		| undefined;
	const documentWithDiscarded = document as Document & {
		wasDiscarded?: boolean;
	};
	const notRestoredReasons =
		"notRestoredReasons" in PerformanceNavigationTiming.prototype &&
		navigationEntry &&
		"notRestoredReasons" in navigationEntry
			? JSON.stringify(
					(
						navigationEntry as PerformanceNavigationTiming & {
							notRestoredReasons?: unknown;
						}
					).notRestoredReasons ?? null,
				)
			: "unavailable";

	return [
		`nav_type=${navigationEntry?.type ?? "unavailable"}`,
		`nav_redirects=${navigationEntry?.redirectCount ?? "unavailable"}`,
		`nav_transfer_size=${navigationEntry?.transferSize ?? "unavailable"}`,
		`document_referrer=${document.referrer || "none"}`,
		`history_length=${window.history.length}`,
		`was_discarded=${documentWithDiscarded.wasDiscarded ?? false}`,
		`not_restored_reasons=${notRestoredReasons}`,
	].join(" ");
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

	const tauriInternals = (
		window as Window & {
			__TAURI_INTERNALS__?: {
				convertFileSrc?: (path: string, protocol?: string) => string;
			};
			ipc?: {
				postMessage?: unknown;
			};
		}
	).__TAURI_INTERNALS__;
	const ipcBridge = (
		window as Window & {
			ipc?: {
				postMessage?: unknown;
			};
		}
	).ipc;

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
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
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
