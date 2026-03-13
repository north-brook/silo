"use client";

import { useMutation } from "@tanstack/react-query";
import { ArrowUp, ChevronsUpDown, Laptop, Terminal } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { invoke } from "../../lib/invoke";
import { Popover, PopoverContent, PopoverTrigger } from "../components/popover";
import { TerminalLoader } from "../components/terminal-loader";
import { ClaudeIcon } from "../icons/claude";
import { CodexIcon } from "../icons/codex";

type Provider = {
	id: "codex" | "claude";
	label: string;
	icon: React.ReactNode;
};
const PROVIDERS = [
	{
		id: "codex",
		label: "Codex",
		icon: <CodexIcon height={14} />,
	},
	{
		id: "claude",
		label: "Claude",
		icon: <ClaudeIcon height={14} />,
	},
] as Provider[];

function statusMessage(status: string): string {
	switch (status) {
		case "STAGING":
		case "PROVISIONING":
			return "Starting workspace...";
		case "STOPPING":
		case "SUSPENDING":
			return "Stopping workspace...";
		case "TERMINATED":
		case "STOPPED":
			return "Workspace is stopped";
		case "":
			return "Loading...";
		default:
			return status.charAt(0) + status.slice(1).toLowerCase();
	}
}

export function PromptWorkspace({
	isRunning,
	status,
	workspace,
	project,
}: {
	isRunning: boolean;
	status: string;
	workspace: string;
	project: string | null;
}) {
	const router = useRouter();
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const [prompt, setPrompt] = useState("");

	useEffect(() => {
		textareaRef.current?.focus();
	}, []);
	const [provider, setProvider] = useState(PROVIDERS[0]);
	const [providerOpen, setProviderOpen] = useState(false);
	const canSubmit = isRunning && prompt.trim().length > 0;

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ terminal: string }>("terminal_create_terminal", {
				workspace,
			}),
		onSuccess: (result) => {
			router.push(
				`/workspace/terminal?project=${encodeURIComponent(project ?? "")}&workspace=${encodeURIComponent(workspace)}&terminal=${encodeURIComponent(result.terminal)}`,
			);
		},
	});

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="w-full max-w-2xl">
				<div className="rounded-lg border border-border-light bg-surface overflow-hidden">
					<textarea
						ref={textareaRef}
						value={prompt}
						onChange={(e) => {
							setPrompt(e.target.value);
							e.target.style.height = "auto";
							e.target.style.height = `${e.target.scrollHeight}px`;
						}}
						onKeyDown={(e) => {
							if (e.key === "Enter" && !e.shiftKey && canSubmit) {
								e.preventDefault();
							}
						}}
						placeholder="What do you want to do?"
						rows={4}
						className="w-full resize-none bg-transparent border-0 px-4 pt-4 pb-2 text-sm text-text-bright placeholder:text-text-placeholder outline-none focus:border-0 focus:ring-0 min-h-[6rem] max-h-64 overflow-y-auto"
					/>
					<div className="flex items-center justify-between px-3 pb-3">
						<Popover open={providerOpen} onOpenChange={setProviderOpen}>
							<PopoverTrigger asChild>
								<button
									type="button"
									className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text hover:bg-btn-hover rounded transition-colors"
								>
									{provider.icon}
									{provider.label}
									<ChevronsUpDown size={10} className="text-text-placeholder" />
								</button>
							</PopoverTrigger>
							<PopoverContent side="bottom" align="start" className="w-36 p-1">
								{PROVIDERS.map((p) => (
									<button
										key={p.id}
										type="button"
										onClick={() => {
											setProvider(p);
											setProviderOpen(false);
										}}
										className={`flex items-center gap-2 w-full px-2 py-1.5 text-xs rounded transition-colors ${
											p.id === provider.id
												? "text-text-bright bg-btn-hover"
												: "text-text hover:bg-btn-hover hover:text-text-bright"
										}`}
									>
										{p.icon}
										{p.label}
									</button>
								))}
							</PopoverContent>
						</Popover>
						<button
							type="button"
							disabled={!canSubmit}
							className="flex items-center justify-center w-7 h-7 rounded-md bg-white text-bg transition-colors hover:bg-white/80 disabled:opacity-30 disabled:cursor-not-allowed"
						>
							<ArrowUp size={14} strokeWidth={2.5} />
						</button>
					</div>
				</div>
				{isRunning ? (
					<div className="flex items-center gap-3 mt-3">
						<button
							type="button"
							disabled={createTerminal.isPending}
							onClick={() => createTerminal.mutate()}
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors disabled:opacity-50"
						>
							<Terminal size={12} />
							Open Terminal
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							<Laptop size={12} />
							Open Desktop
						</button>
					</div>
				) : (
					<div className="flex items-center gap-2 mt-3 px-2 py-1 text-[11px] text-text-muted">
						<TerminalLoader className="text-text-muted" />
						<span>{statusMessage(status)}</span>
					</div>
				)}
			</div>
		</div>
	);
}
