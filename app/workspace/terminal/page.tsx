"use client";

import { Channel } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useRef } from "react";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "../../../lib/invoke";
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
	terminal: string;
	session: {
		name: string;
		pid: number | null;
		clients: number;
		started_in: string | null;
		created_at: string | null;
	};
	scrollback_vt: string;
	scrollback_truncated: boolean;
}

interface TerminalExitPayload {
	terminal: string;
	exit_code: number;
	signal: string | null;
}

interface TerminalErrorPayload {
	terminal: string;
	message: string;
}

const THEME = {
	background: "#0a0a0c",
	foreground: "#b0b8c8",
	cursor: "#638cff",
	cursorAccent: "#0a0a0c",
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

export default function TerminalPage() {
	return (
		<Suspense>
			<TerminalView />
		</Suspense>
	);
}

function TerminalView() {
	const searchParams = useSearchParams();
	const workspace = searchParams.get("workspace") ?? "";
	const terminal = searchParams.get("terminal") ?? "";

	if (!workspace || !terminal) {
		return null;
	}

	return <WorkspaceTerminal workspace={workspace} terminal={terminal} />;
}

function WorkspaceTerminal({
	workspace,
	terminal,
}: {
	workspace: string;
	terminal: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const terminalRef = useRef<Terminal | null>(null);
	const terminalIdRef = useRef<string | null>(null);
	const pendingDetachRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(() => {
		if (!containerRef.current) return;
		if (pendingDetachRef.current) {
			clearTimeout(pendingDetachRef.current);
			pendingDetachRef.current = null;
		}

		let disposed = false;
		let attachedTerminal: string | null = null;
		let unlistenExit: (() => void) | null = null;
		let unlistenError: (() => void) | null = null;

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
		term.open(containerRef.current);
		fitAddon.fit();
		term.focus();

		terminalRef.current = term;
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
			if (disposed) return;
			term.write(normalizeTerminalOutput(data));
		};

		void getCurrentWebviewWindow()
			.listen<TerminalExitPayload>("terminal://exit", ({ payload }) => {
				if (!disposed && payload.terminal === terminalIdRef.current) {
					const reason = payload.signal
						? `signal=${payload.signal}`
						: `exit=${payload.exit_code}`;
					term.writeln(`\r\n[terminal exited: ${reason}]`);
				}
			})
			.then((unlisten) => {
				if (disposed) {
					void unlisten();
					return;
				}
				unlistenExit = unlisten;
			});

		void getCurrentWebviewWindow()
			.listen<TerminalErrorPayload>("terminal://error", ({ payload }) => {
				if (!disposed && payload.terminal === terminalIdRef.current) {
					term.writeln(`\r\n[terminal error] ${payload.message}`);
				}
			})
			.then((unlisten) => {
				if (disposed) {
					void unlisten();
					return;
				}
				unlistenError = unlisten;
			});

		void (async () => {
			try {
				const result = await invoke<TerminalAttachResult>(
					"terminal_attach_terminal",
					{
						workspace,
						name: terminal,
						output,
					},
				);
				if (disposed) return;

				attachedTerminal = result.terminal;
				terminalIdRef.current = result.terminal;

				if (result.scrollback_vt) {
					term.write(result.scrollback_vt);
				}

				await invoke("terminal_resize_terminal", {
					terminal: result.terminal,
					cols: term.cols,
					rows: term.rows,
				});
				term.focus();
			} catch (error) {
				if (!disposed) {
					term.writeln(`\r\n[attach failed] ${String(error)}`);
				}
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
			if (terminalIdRef.current) {
				void invoke("terminal_resize_terminal", {
					terminal: terminalIdRef.current,
					cols,
					rows,
				});
			}
		});

		const resizeObserver = new ResizeObserver(() => {
			fitAddon.fit();
		});
		resizeObserver.observe(containerRef.current);

		return () => {
			disposed = true;
			resizeObserver.disconnect();
			detachBindings();
			term.dispose();
			terminalRef.current = null;
			if (unlistenExit) {
				void unlistenExit();
			}
			if (unlistenError) {
				void unlistenError();
			}
			if (attachedTerminal) {
				pendingDetachRef.current = setTimeout(() => {
					void invoke("terminal_detach_terminal", {
						workspace,
						name: terminal,
					});
					if (terminalIdRef.current === attachedTerminal) {
						terminalIdRef.current = null;
					}
					pendingDetachRef.current = null;
				}, 250);
			}
		};
	}, [workspace, terminal]);

	return <div ref={containerRef} className="flex-1 min-h-0" />;
}
