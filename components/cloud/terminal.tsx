"use client";

import { useQueryClient } from "@tanstack/react-query";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "../../lib/invoke";
import type { WorkspaceSession } from "../../lib/workspaces";
import type { CloudSession } from "../../lib/cloud";
import { attachTerminalBindings } from "./bindings";

const DELETE_BYTE = 0x7f;
const BACKSPACE_ERASE_SEQUENCE = [0x08, 0x20, 0x08];

function normalizeTerminalOutput(data: ArrayBuffer): Uint8Array {
	const bytes = new Uint8Array(data);
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

interface TerminalAttachResult {
	terminal_id: string;
	session: WorkspaceSession;
	scrollback_vt: string;
	scrollback_truncated: boolean;
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

type HostStatus = "idle" | "attaching" | "ready" | "error";

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

export function CloudTerminalHost({
	session,
	target,
	visible,
	skipInitialScrollback,
	onFreshConsumed,
	onHostStateChange,
}: {
	session: CloudSession;
	target: HTMLElement | null;
	visible: boolean;
	skipInitialScrollback: boolean;
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
	const attachedRef = useRef(false);
	const initialSkipScrollbackRef = useRef(skipInitialScrollback);
	const onFreshConsumedRef = useRef(onFreshConsumed);
	const onHostStateChangeRef = useRef(onHostStateChange);
	const visibleRef = useRef(visible);
	const [isMountReady, setIsMountReady] = useState(false);
	const isPageForeground = usePageIsForeground();

	useEffect(() => {
		visibleRef.current = visible;
	}, [visible]);

	const fitAndResizeTerminal = () => {
		if (!visibleRef.current) {
			return;
		}

		const fitAddon = fitAddonRef.current;
		const terminal = termRef.current;
		if (!fitAddon || !terminal) {
			return;
		}

		fitAddon.fit();
	};

	const scheduleFitAndResize = () => {
		requestAnimationFrame(() => {
			fitAndResizeTerminal();
			requestAnimationFrame(() => {
				fitAndResizeTerminal();
			});
		});
	};

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
	}, [target]);

	useEffect(() => {
		const mountElement = mountElementRef.current;
		if (!mountElement || !isMountReady || attachedRef.current) {
			return;
		}

		attachedRef.current = true;
		onHostStateChangeRef.current({ status: "attaching", errorMessage: null });
		console.info("cloud terminal attach start", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			skipInitialScrollback: initialSkipScrollbackRef.current,
		});

		let disposed = false;
		let unlistenExit: (() => void | Promise<void>) | null = null;
		let unlistenError: (() => void | Promise<void>) | null = null;

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
			if (!terminalIdRef.current) {
				return;
			}

			const bytes = typeof data === "string" ? encoder.encode(data) : data;
			void invoke("terminal_write_terminal", {
				terminal: terminalIdRef.current,
				data: Array.from(bytes),
			});
		};
		const detachBindings = attachTerminalBindings(term, sendTerminalInput);

		const output = new Channel<ArrayBuffer>();
		output.onmessage = (data: ArrayBuffer) => {
			if (disposed) {
				return;
			}
			term.write(normalizeTerminalOutput(data));
		};

		void getCurrentWindow()
			.listen<TerminalExitPayload>("terminal://exit", ({ payload }) => {
				if (!disposed && payload.terminal_id === terminalIdRef.current) {
					const reason = payload.signal
						? `signal=${payload.signal}`
						: `exit=${payload.exit_code}`;
					term.writeln(`\r\n[terminal exited: ${reason}]`);
				}
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
				if (!disposed && payload.terminal_id === terminalIdRef.current) {
					term.writeln(`\r\n[terminal error] ${payload.message}`);
				}
			})
			.then((unlisten) => {
				if (disposed) {
					void Promise.resolve(unlisten()).catch(() => {});
					return;
				}
				unlistenError = unlisten;
			});

		void (async () => {
			try {
				const result = await invoke<TerminalAttachResult>(
					"terminal_attach_terminal",
					{
						workspace: session.workspace,
						attachmentId: session.attachmentId,
						skipScrollback: initialSkipScrollbackRef.current,
						output,
					},
				);
				if (disposed) {
					return;
				}

				terminalIdRef.current = result.terminal_id;
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

				if (result.scrollback_vt) {
					term.write(result.scrollback_vt);
				}

				if (initialSkipScrollbackRef.current) {
					initialSkipScrollbackRef.current = false;
					onFreshConsumedRef.current();
				}

				if (visibleRef.current) {
					scheduleFitAndResize();
					term.focus();
				}
			} catch (error) {
				if (disposed) {
					return;
				}
				term.writeln(`\r\n[attach failed] ${String(error)}`);
				console.warn("cloud terminal attach failed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					error: String(error),
				});
				onHostStateChangeRef.current({
					status: "error",
					errorMessage: String(error),
				});
			}
		})();

		term.onData((data) => {
			sendTerminalInput(data);
		});

		term.onBinary((data) => {
			const bytes = new Uint8Array(data.length);
			for (let i = 0; i < data.length; i++) {
				bytes[i] = data.charCodeAt(i);
			}
			sendTerminalInput(bytes);
		});

		term.onResize(({ cols, rows }) => {
			if (!terminalIdRef.current) {
				return;
			}
			const lastResize = lastResizeRef.current;
			if (lastResize && lastResize.cols === cols && lastResize.rows === rows) {
				return;
			}
			lastResizeRef.current = { cols, rows };

			void invoke("terminal_resize_terminal", {
				terminal: terminalIdRef.current,
				cols,
				rows,
			});
		});

		const resizeObserver = new ResizeObserver(() => {
			scheduleFitAndResize();
		});
		resizeObserver.observe(mountElement);
		if (target) {
			resizeObserver.observe(target);
		}

		return () => {
			disposed = true;
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
			if (terminalIdRef.current) {
				void invoke("terminal_detach_terminal", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
				});
				terminalIdRef.current = null;
			}
			lastResizeRef.current = null;
			if (mountElement.parentElement) {
				mountElement.parentElement.removeChild(mountElement);
			}
		};
	}, [
		isMountReady,
		session.attachmentId,
		session.workspace,
	]);

	useEffect(() => {
		if (!visible) {
			return;
		}

		scheduleFitAndResize();
		termRef.current?.focus();
	}, [target, visible]);

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
