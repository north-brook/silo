import { useQueryClient } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useCallback, useEffect, useRef, useState } from "react";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "@/shared/lib/invoke";
import { domFocusSnapshot } from "@/shared/lib/focus-debug";
import type { WorkspaceSession } from "@/workspaces/api";
import type { CloudSession } from "@/workspaces/hosts/model";
import { attachTerminalBindings } from "./bindings";
import {
	isRetryableTerminalTransportMessage,
	reconnectDelayMs,
} from "./reconnect";
import {
	isMissingLocalTerminalAttachmentMessage,
	STALE_ATTACHMENT_RECONNECT_MESSAGE,
	STALE_ATTACHMENT_RESUME_NOTICE,
} from "./recovery";
import { assistantTerminalModel } from "./session";

const DELETE_BYTE = 0x7f;
const BACKSPACE_ERASE_SEQUENCE = [0x08, 0x20, 0x08];
const MAX_AUTO_RECONNECT_ATTEMPTS = 5;
const MIN_ATTACH_COLS = 10;
const MIN_ATTACH_ROWS = 4;
const MIN_ATTACH_PIXEL_SIZE = 4;

function normalizeTerminalOutput(data: ArrayBuffer | Uint8Array): Uint8Array {
	const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
	if (!bytes.includes(DELETE_BYTE)) {
		return bytes;
	}

	const normalized: number[] = [];
	for (const byte of bytes) {
		if (byte === DELETE_BYTE) {
			normalized.push(...BACKSPACE_ERASE_SEQUENCE);
			continue;
		}
		normalized.push(byte);
	}

	return Uint8Array.from(normalized);
}

function writeTerminalOutput(term: Terminal, data: Uint8Array): Promise<void> {
	if (data.length === 0) {
		return Promise.resolve();
	}
	return new Promise((resolve) => {
		term.write(data, resolve);
	});
}

interface TerminalAttachResult {
	terminal_id: string;
	session: WorkspaceSession;
	initial_output: number[];
}

interface TerminalSize {
	cols: number;
	rows: number;
}

interface TerminalExitPayload {
	terminal_id: string;
	exit_code: number;
	signal: string | null;
}

interface TerminalErrorPayload {
	terminal_id: string;
	message: string;
}

interface TerminalDisconnectPayload {
	terminal_id: string;
	message: string;
}

interface TerminalProbeResult {
	exists: boolean;
}

type HostStatus = "idle" | "attaching" | "ready" | "reconnecting" | "error";

function usePageIsForeground() {
	const [isForeground, setIsForeground] = useState(() => {
		if (typeof document === "undefined") {
			return true;
		}
		return document.visibilityState === "visible" && document.hasFocus();
	});

	useEffect(() => {
		const updateForeground = () => {
			setIsForeground(
				document.visibilityState === "visible" && document.hasFocus(),
			);
		};

		updateForeground();
		window.addEventListener("focus", updateForeground);
		window.addEventListener("blur", updateForeground);
		document.addEventListener("visibilitychange", updateForeground);

		return () => {
			window.removeEventListener("focus", updateForeground);
			window.removeEventListener("blur", updateForeground);
			document.removeEventListener("visibilitychange", updateForeground);
		};
	}, []);

	return isForeground;
}

const THEME = {
	background: "#131419",
	foreground: "#b0b8c8",
	cursor: "#638cff",
	cursorAccent: "#131419",
	selectionBackground: "#638cff3d",
	black: "#16181e",
	red: "#f87171",
	green: "#34d399",
	yellow: "#fbbf24",
	blue: "#638cff",
	magenta: "#c084fc",
	cyan: "#22d3ee",
	white: "#b0b8c8",
	brightBlack: "#4a5068",
	brightRed: "#fca5a5",
	brightGreen: "#6ee7b7",
	brightYellow: "#fde68a",
	brightBlue: "#7a9fff",
	brightMagenta: "#d8b4fe",
	brightCyan: "#67e8f9",
	brightWhite: "#e2e6ee",
};

export function TerminalSessionHost({
	session,
	target,
	visible,
	skipInitialScrollback,
	retryNonce,
	onFreshConsumed,
	onHostStateChange,
}: {
	session: CloudSession;
	target: HTMLElement | null;
	visible: boolean;
	skipInitialScrollback: boolean;
	retryNonce: number;
	onFreshConsumed: () => void;
	onHostStateChange: (state: {
		status: HostStatus;
		errorMessage?: string | null;
		terminalId?: string | null;
	}) => void;
}) {
	const queryClient = useQueryClient();
	const terminalIdRef = useRef<string | null>(null);
	const lastResizeRef = useRef<{ cols: number; rows: number } | null>(null);
	const mountElementRef = useRef<HTMLDivElement | null>(null);
	const termRef = useRef<Terminal | null>(null);
	const fitAddonRef = useRef<FitAddon | null>(null);
	const pendingMarkReadRef = useRef(false);
	const initialSkipScrollbackRef = useRef(skipInitialScrollback);
	const onFreshConsumedRef = useRef(onFreshConsumed);
	const onHostStateChangeRef = useRef(onHostStateChange);
	const visibleRef = useRef(visible);
	const reconnectTimeoutRef = useRef<number | null>(null);
	const reconnectAttemptRef = useRef(0);
	const reconnectMessageRef = useRef<string | null>(null);
	const pendingReconnectRef = useRef(false);
	const probeInFlightRef = useRef<string | null>(null);
	const staleAttachmentRecoveryRef = useRef<
		(source: string, detail?: string) => void
	>(() => {});
	const attachInFlightRef = useRef(false);
	const attachQueuedRef = useRef(false);
	const attachSizeRef = useRef<TerminalSize | null>(null);
	const retryNonceRef = useRef(retryNonce);
	const isAssistantSessionRef = useRef(
		assistantTerminalModel(session.name) != null,
	);
	const [isMountReady, setIsMountReady] = useState(false);
	const [attachNonce, setAttachNonce] = useState(0);
	const isPageForeground = usePageIsForeground();

	useEffect(() => {
		visibleRef.current = visible;
	}, [visible]);

	useEffect(() => {
		isAssistantSessionRef.current =
			assistantTerminalModel(session.name) != null;
	}, [session.name]);

	const clearReconnectTimer = useCallback(() => {
		if (reconnectTimeoutRef.current !== null) {
			window.clearTimeout(reconnectTimeoutRef.current);
			reconnectTimeoutRef.current = null;
		}
	}, []);

	const fitAndResizeTerminal = useCallback(() => {
		if (!visibleRef.current) {
			return null;
		}

		const fitAddon = fitAddonRef.current;
		const terminal = termRef.current;
		const mountElement = mountElementRef.current;
		if (!fitAddon || !terminal || !mountElement) {
			return null;
		}

		fitAddon.fit();

		const bounds = mountElement.getBoundingClientRect();
		if (
			bounds.width <= MIN_ATTACH_PIXEL_SIZE ||
			bounds.height <= MIN_ATTACH_PIXEL_SIZE ||
			terminal.cols < MIN_ATTACH_COLS ||
			terminal.rows < MIN_ATTACH_ROWS
		) {
			return null;
		}

		const style = window.getComputedStyle(mountElement);
		const paddingV =
			(parseFloat(style.paddingTop) || 0) +
			(parseFloat(style.paddingBottom) || 0);
		if (paddingV > 0) {
			const core = (terminal as any)._core;
			const cellHeight = core?._renderService?.dimensions?.css?.cell?.height;
			if (cellHeight > 0) {
				const maxRows = Math.max(
					MIN_ATTACH_ROWS,
					Math.floor((bounds.height - paddingV) / cellHeight),
				);
				if (terminal.rows > maxRows) {
					terminal.resize(terminal.cols, maxRows);
				}
			}
		}

		return {
			cols: terminal.cols,
			rows: terminal.rows,
		} satisfies TerminalSize;
	}, []);

	const scheduleFitAndResize = useCallback(() => {
		requestAnimationFrame(() => {
			fitAndResizeTerminal();
			requestAnimationFrame(() => {
				fitAndResizeTerminal();
			});
		});
	}, [fitAndResizeTerminal]);

	const sendResize = useCallback(
		(cols: number, rows: number) => {
			const terminalId = terminalIdRef.current;
			if (!terminalId) {
				return;
			}

			const next = { cols, rows };
			const lastResize = lastResizeRef.current;
			if (
				lastResize &&
				lastResize.cols === next.cols &&
				lastResize.rows === next.rows
			) {
				return;
			}
			lastResizeRef.current = next;

			void invoke("terminal_resize_terminal", {
				terminal: terminalId,
				cols: next.cols,
				rows: next.rows,
			}).catch((error) => {
				const message = String(error);
				if (
					terminalIdRef.current === terminalId &&
					isMissingLocalTerminalAttachmentMessage(message)
				) {
					staleAttachmentRecoveryRef.current("resize", message);
					return;
				}
				console.warn("cloud terminal resize failed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					terminalId,
					error: message,
				});
			});
		},
		[session.attachmentId, session.workspace],
	);

	const triggerAttach = useCallback(() => {
		if (
			!isMountReady ||
			!visibleRef.current ||
			!termRef.current ||
			attachQueuedRef.current ||
			attachInFlightRef.current ||
			terminalIdRef.current
		) {
			return;
		}
		const size = fitAndResizeTerminal();
		if (!size) {
			return;
		}
		attachQueuedRef.current = true;
		attachSizeRef.current = size;
		setAttachNonce((previous) => previous + 1);
	}, [fitAndResizeTerminal, isMountReady]);

	const scheduleReconnectAttempt = useCallback(() => {
		if (
			!pendingReconnectRef.current ||
			attachInFlightRef.current ||
			reconnectTimeoutRef.current !== null
		) {
			return;
		}
		if (reconnectAttemptRef.current >= MAX_AUTO_RECONNECT_ATTEMPTS) {
			pendingReconnectRef.current = false;
			onHostStateChangeRef.current({
				status: "error",
				errorMessage:
					reconnectMessageRef.current ??
					"Connection lost. Automatic reconnect attempts failed.",
				terminalId: null,
			});
			return;
		}
		if (!visibleRef.current) {
			return;
		}

		reconnectAttemptRef.current += 1;
		const delay = reconnectDelayMs(reconnectAttemptRef.current);
		onHostStateChangeRef.current({
			status: "reconnecting",
			errorMessage: reconnectMessageRef.current,
			terminalId: null,
		});
		reconnectTimeoutRef.current = window.setTimeout(() => {
			reconnectTimeoutRef.current = null;
			triggerAttach();
		}, delay);
	}, [triggerAttach]);

	const requestReconnect = useCallback(
		(message: string) => {
			terminalIdRef.current = null;
			pendingReconnectRef.current = true;
			reconnectMessageRef.current = message;
			onHostStateChangeRef.current({
				status: "reconnecting",
				errorMessage: message,
				terminalId: null,
			});
			scheduleReconnectAttempt();
		},
		[scheduleReconnectAttempt],
	);

	const recoverFromStaleAttachment = useCallback(
		(source: string, detail?: string) => {
			const terminalId = terminalIdRef.current;
			if (!terminalId || pendingReconnectRef.current) {
				return;
			}

			console.warn("stale terminal attachment detected", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
				terminalId,
				source,
				detail,
			});
			lastResizeRef.current = null;
			termRef.current?.writeln(`\r\n${STALE_ATTACHMENT_RESUME_NOTICE}`);
			requestReconnect(STALE_ATTACHMENT_RECONNECT_MESSAGE);
		},
		[requestReconnect, session.attachmentId, session.workspace],
	);

	const verifyCurrentAttachment = useCallback(
		async (source: string) => {
			const terminalId = terminalIdRef.current;
			if (
				!terminalId ||
				!visibleRef.current ||
				!termRef.current ||
				attachInFlightRef.current ||
				pendingReconnectRef.current ||
				probeInFlightRef.current === terminalId
			) {
				return;
			}

			probeInFlightRef.current = terminalId;
			try {
				const result = await invoke<TerminalProbeResult>(
					"terminal_probe_terminal",
					{
						terminal: terminalId,
					},
				);
				if (!result.exists && terminalIdRef.current === terminalId) {
					recoverFromStaleAttachment(
						source,
						"probe reported missing attachment",
					);
				}
			} catch (error) {
				const message = String(error);
				if (
					terminalIdRef.current === terminalId &&
					isMissingLocalTerminalAttachmentMessage(message)
				) {
					recoverFromStaleAttachment(source, message);
					return;
				}
				console.warn("terminal attachment probe failed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					terminalId,
					source,
					error: message,
				});
			} finally {
				if (probeInFlightRef.current === terminalId) {
					probeInFlightRef.current = null;
				}
			}
		},
		[recoverFromStaleAttachment, session.attachmentId, session.workspace],
	);

	if (typeof document !== "undefined" && !mountElementRef.current) {
		const element = document.createElement("div");
		element.className = "h-full w-full p-1.5";
		mountElementRef.current = element;
	}

	useEffect(() => {
		onFreshConsumedRef.current = onFreshConsumed;
	}, [onFreshConsumed]);

	useEffect(() => {
		onHostStateChangeRef.current = onHostStateChange;
	}, [onHostStateChange]);

	useEffect(() => {
		staleAttachmentRecoveryRef.current = recoverFromStaleAttachment;
	}, [recoverFromStaleAttachment]);

	useEffect(() => {
		const mountElement = mountElementRef.current;
		if (!mountElement || !target) {
			return;
		}

		if (mountElement.parentElement !== target) {
			target.appendChild(mountElement);
		}
		setIsMountReady(true);
		if (visibleRef.current) {
			scheduleFitAndResize();
		}
	}, [scheduleFitAndResize, target]);

	useEffect(() => {
		const mountElement = mountElementRef.current;
		if (!mountElement || !isMountReady) {
			return;
		}

		let disposed = false;
		let unlistenExit: (() => void | Promise<void>) | null = null;
		let unlistenError: (() => void | Promise<void>) | null = null;
		let unlistenDisconnect: (() => void | Promise<void>) | null = null;

		const term = new Terminal({
			theme: THEME,
			fontFamily:
				'"SF Mono", "Fira Code", "JetBrains Mono", "Cascadia Code", ui-monospace, monospace',
			fontSize: 13,
			lineHeight: 1.2,
			cursorBlink: true,
			cursorStyle: "bar",
			allowTransparency: true,
			scrollback: 10000,
		});
		const fitAddon = new FitAddon();
		term.loadAddon(fitAddon);
		term.open(mountElement);

		termRef.current = term;
		fitAddonRef.current = fitAddon;

		const encoder = new TextEncoder();
		const sendTerminalInput = (data: string | Uint8Array) => {
			const terminalId = terminalIdRef.current;
			if (!terminalId) {
				return;
			}

			const bytes = typeof data === "string" ? encoder.encode(data) : data;
			void invoke("terminal_write_terminal", {
				terminal: terminalId,
				data: Array.from(bytes),
			}).catch((error) => {
				const message = String(error);
				if (disposed || terminalIdRef.current !== terminalId) {
					return;
				}
				if (isMissingLocalTerminalAttachmentMessage(message)) {
					recoverFromStaleAttachment("input", message);
					return;
				}
				console.warn("cloud terminal input failed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					terminalId,
					error: message,
				});
			});
		};
		const detachBindings = attachTerminalBindings(term, sendTerminalInput, {
			isAssistantSession: () => isAssistantSessionRef.current,
		});

		term.onData((data) => {
			sendTerminalInput(data);
		});

		term.onBinary((data) => {
			const bytes = new Uint8Array(data.length);
			for (let index = 0; index < data.length; index++) {
				bytes[index] = data.charCodeAt(index);
			}
			sendTerminalInput(bytes);
		});

		term.onResize(({ cols, rows }) => {
			sendResize(cols, rows);
		});

		void getCurrentWindow()
			.listen<TerminalExitPayload>("terminal://exit", ({ payload }) => {
				if (disposed || payload.terminal_id !== terminalIdRef.current) {
					return;
				}

				terminalIdRef.current = null;
				clearReconnectTimer();
				pendingReconnectRef.current = false;
				reconnectAttemptRef.current = 0;
				reconnectMessageRef.current = null;
				const reason = payload.signal
					? `signal=${payload.signal}`
					: `exit=${payload.exit_code}`;
				term.writeln(`\r\n[terminal exited: ${reason}]`);
				onHostStateChangeRef.current({
					status: "ready",
					errorMessage: null,
					terminalId: null,
				});
			})
			.then((unlisten) => {
				if (disposed) {
					void Promise.resolve(unlisten()).catch(() => {});
					return;
				}
				unlistenExit = unlisten;
			});

		void getCurrentWindow()
			.listen<TerminalErrorPayload>("terminal://error", ({ payload }) => {
				if (disposed || payload.terminal_id !== terminalIdRef.current) {
					return;
				}

				term.writeln(`\r\n[terminal error] ${payload.message}`);
			})
			.then((unlisten) => {
				if (disposed) {
					void Promise.resolve(unlisten()).catch(() => {});
					return;
				}
				unlistenError = unlisten;
			});

		void getCurrentWindow()
			.listen<TerminalDisconnectPayload>(
				"terminal://disconnect",
				({ payload }) => {
					if (disposed || payload.terminal_id !== terminalIdRef.current) {
						return;
					}

					term.writeln("\r\n[connection lost, attempting to resume]");
					requestReconnect(payload.message);
				},
			)
			.then((unlisten) => {
				if (disposed) {
					void Promise.resolve(unlisten()).catch(() => {});
					return;
				}
				unlistenDisconnect = unlisten;
			});

		const resizeObserver = new ResizeObserver(() => {
			scheduleFitAndResize();
			if (!terminalIdRef.current) {
				triggerAttach();
			}
		});
		resizeObserver.observe(mountElement);

		return () => {
			disposed = true;
			clearReconnectTimer();
			resizeObserver.disconnect();
			detachBindings();
			term.dispose();
			termRef.current = null;
			fitAddonRef.current = null;
			if (unlistenExit) {
				void Promise.resolve(unlistenExit()).catch(() => {});
			}
			if (unlistenError) {
				void Promise.resolve(unlistenError()).catch(() => {});
			}
			if (unlistenDisconnect) {
				void Promise.resolve(unlistenDisconnect()).catch(() => {});
			}
			if (mountElement.parentElement) {
				mountElement.parentElement.removeChild(mountElement);
			}
		};
	}, [
		clearReconnectTimer,
		isMountReady,
		recoverFromStaleAttachment,
		requestReconnect,
		scheduleFitAndResize,
		sendResize,
		triggerAttach,
	]);

	useEffect(() => {
		if (!isMountReady || !termRef.current) {
			return;
		}
		const requestedSize = attachSizeRef.current;
		if (!requestedSize) {
			return;
		}

		let disposed = false;
		let attachedTerminalId: string | null = null;
		const term = termRef.current;
		const attachRunKey = attachNonce;
		const shouldReconnect = pendingReconnectRef.current;

		attachQueuedRef.current = false;
		attachInFlightRef.current = true;
		onHostStateChangeRef.current({
			status: shouldReconnect ? "reconnecting" : "attaching",
			errorMessage: shouldReconnect ? reconnectMessageRef.current : null,
			terminalId: null,
		});
		console.info("cloud terminal attach start", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			attachNonce: attachRunKey,
			cols: requestedSize.cols,
			rows: requestedSize.rows,
			reconnectAttempt: reconnectAttemptRef.current,
		});

		const output = new Channel<ArrayBuffer>();
		output.onmessage = (data: ArrayBuffer) => {
			if (disposed) {
				return;
			}
			term.write(normalizeTerminalOutput(data));
		};

		void (async () => {
			try {
				const result = await invoke<TerminalAttachResult>(
					"terminal_attach_terminal",
					{
						workspace: session.workspace,
						attachmentId: session.attachmentId,
						cols: requestedSize.cols,
						rows: requestedSize.rows,
						output,
					},
				);
				if (disposed) {
					return;
				}

				attachInFlightRef.current = false;
				terminalIdRef.current = result.terminal_id;
				attachedTerminalId = result.terminal_id;
				clearReconnectTimer();
				pendingReconnectRef.current = false;
				reconnectAttemptRef.current = 0;
				reconnectMessageRef.current = null;
				onHostStateChangeRef.current({
					status: "ready",
					errorMessage: null,
					terminalId: result.terminal_id,
				});
				console.info("cloud terminal attach ready", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					terminalId: result.terminal_id,
				});

				term.reset();
				lastResizeRef.current = null;
				await writeTerminalOutput(
					term,
					normalizeTerminalOutput(Uint8Array.from(result.initial_output)),
				);
				await invoke("terminal_finish_attach", {
					terminal: result.terminal_id,
				});
				if (disposed) {
					return;
				}

				if (initialSkipScrollbackRef.current) {
					initialSkipScrollbackRef.current = false;
					onFreshConsumedRef.current();
				}

				if (visibleRef.current) {
					scheduleFitAndResize();
					term.focus();
				}
				sendResize(term.cols, term.rows);
			} catch (error) {
				attachInFlightRef.current = false;
				if (disposed) {
					return;
				}

				const message = String(error);
				term.writeln(`\r\n[attach failed] ${message}`);
				console.warn("cloud terminal attach failed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					error: message,
				});

				if (isRetryableTerminalTransportMessage(message)) {
					requestReconnect(message);
					return;
				}

				pendingReconnectRef.current = false;
				reconnectAttemptRef.current = 0;
				reconnectMessageRef.current = null;
				onHostStateChangeRef.current({
					status: "error",
					errorMessage: message,
					terminalId: null,
				});
			}
		})();

		return () => {
			disposed = true;
			attachQueuedRef.current = false;
			attachInFlightRef.current = false;
			if (attachedTerminalId && terminalIdRef.current === attachedTerminalId) {
				void invoke("terminal_detach_terminal", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
				});
				terminalIdRef.current = null;
			}
		};
	}, [
		attachNonce,
		clearReconnectTimer,
		isMountReady,
		requestReconnect,
		scheduleFitAndResize,
		sendResize,
		session.attachmentId,
		session.workspace,
	]);

	useEffect(() => {
		if (!visible) {
			return;
		}

		console.info("terminal focus requested", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			terminalId: terminalIdRef.current,
			isPageForeground,
			...domFocusSnapshot(),
		});
		scheduleFitAndResize();
		termRef.current?.focus();
		window.requestAnimationFrame(() => {
			console.info("terminal focus settled", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
				terminalId: terminalIdRef.current,
				isPageForeground,
				...domFocusSnapshot(),
			});
		});
		if (!terminalIdRef.current) {
			triggerAttach();
		}
		if (pendingReconnectRef.current && !terminalIdRef.current) {
			scheduleReconnectAttempt();
		}
	}, [scheduleFitAndResize, scheduleReconnectAttempt, triggerAttach, visible]);

	useEffect(() => {
		if (!visible || !isPageForeground) {
			return;
		}

		void verifyCurrentAttachment("page resume");
	}, [isPageForeground, verifyCurrentAttachment, visible]);

	useEffect(() => {
		if (retryNonceRef.current === retryNonce) {
			return;
		}

		retryNonceRef.current = retryNonce;
		clearReconnectTimer();
		reconnectAttemptRef.current = 0;
		reconnectMessageRef.current = null;
		pendingReconnectRef.current = false;
		terminalIdRef.current = null;
		triggerAttach();
	}, [clearReconnectTimer, retryNonce, triggerAttach]);

	useEffect(() => {
		if (!visible || !isPageForeground || session.unread !== true) {
			pendingMarkReadRef.current = false;
			return;
		}

		if (pendingMarkReadRef.current) {
			return;
		}

		pendingMarkReadRef.current = true;
		void invoke("terminal_read_terminal", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
		})
			.then(() =>
				Promise.all([
					queryClient.invalidateQueries({
						queryKey: ["terminal_list_terminals", session.workspace],
					}),
					queryClient.invalidateQueries({
						queryKey: ["workspaces_get_workspace", session.workspace],
					}),
				]),
			)
			.finally(() => {
				pendingMarkReadRef.current = false;
			});
	}, [
		isPageForeground,
		queryClient,
		session.attachmentId,
		session.unread,
		session.workspace,
		visible,
	]);

	return null;
}
