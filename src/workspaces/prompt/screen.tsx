"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowUp, ChevronsUpDown, Globe, Terminal } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import {
	type PromptProviderId,
	usePromptDraft,
} from "@/workspaces/prompt/draft";
import { toast } from "@/shared/ui/toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cloudSessionHref } from "@/workspaces/routes/paths";
import { invoke } from "@/shared/lib/invoke";
import { submitWorkspacePrompt } from "@/workspaces/api";

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
	ready,
	status,
	workspace,
	project,
}: {
	isRunning: boolean;
	ready: boolean;
	status: string;
	workspace: string;
	project: string | null;
}) {
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const { clearDraft, prompt, providerId, setPrompt, setProviderId } =
		usePromptDraft(workspace);

	useEffect(() => {
		const textarea = textareaRef.current;
		if (!textarea) {
			return;
		}

		textarea.dataset.workspace = workspace;
		textarea.focus();
		const caret = textarea.value.length;
		textarea.setSelectionRange(caret, caret);
	}, [workspace]);
	const [providerOpen, setProviderOpen] = useState(false);
	const provider =
		PROVIDERS.find((candidate) => candidate.id === providerId) ?? PROVIDERS[0];

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ attachment_id: string }>("terminal_create_terminal", {
				workspace,
			}),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				cloudSessionHref({
					project: project ?? "",
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create terminal",
				description: error.message,
			});
		},
	});
	const submitPrompt = useMutation({
		mutationFn: () => submitWorkspacePrompt(workspace, prompt, provider.id),
		onSuccess: (result) => {
			clearDraft();
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				cloudSessionHref({
					project: project ?? "",
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to submit prompt",
				description: error.message,
			});
		},
	});
	const canSubmit =
		isRunning && ready && prompt.trim().length > 0 && !submitPrompt.isPending;

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="w-full max-w-2xl">
				<div className="flex justify-center mb-6">
					<SiloIcon height={32} />
				</div>
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
							if (e.key === "Tab" && e.shiftKey) {
								e.preventDefault();
								const idx = PROVIDERS.findIndex((p) => p.id === provider.id);
								const nextProvider =
									PROVIDERS[(idx + 1) % PROVIDERS.length]?.id ?? "codex";
								setProviderId(nextProvider as PromptProviderId);
							}
							if (e.key === "Enter" && !e.shiftKey && canSubmit) {
								e.preventDefault();
								submitPrompt.mutate();
							}
						}}
						placeholder="What should we ship?"
						rows={4}
						spellCheck={false}
						autoCorrect="off"
						autoCapitalize="off"
						className="w-full resize-none bg-transparent border-0 px-4 pt-4 pb-2 text-sm text-text-bright placeholder:text-text-placeholder outline-none focus:border-0 focus:ring-0 min-h-[6rem] max-h-64 overflow-y-auto"
					/>
					<div className="flex items-center justify-between px-3 pb-3">
						<Tooltip>
							<TooltipTrigger asChild>
								<div>
									<Popover open={providerOpen} onOpenChange={setProviderOpen}>
										<PopoverTrigger asChild>
											<button
												type="button"
												className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text hover:bg-btn-hover rounded transition-colors"
											>
												{provider.icon}
												{provider.label}
												<ChevronsUpDown
													size={10}
													className="text-text-placeholder"
												/>
											</button>
										</PopoverTrigger>
										<PopoverContent
											side="bottom"
											align="start"
											className="w-36 p-1"
										>
											{PROVIDERS.map((p) => (
												<button
													key={p.id}
													type="button"
													onClick={() => {
														setProviderId(p.id);
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
								</div>
							</TooltipTrigger>
							<TooltipContent side="right">
								<span className="flex items-center gap-1.5">
									Toggle model
									<span className="flex items-center gap-0.5">
										<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
											⇧
										</kbd>
										<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
											⇥
										</kbd>
									</span>
								</span>
							</TooltipContent>
						</Tooltip>
						<button
							type="button"
							disabled={!canSubmit}
							onClick={() => submitPrompt.mutate()}
							className="flex items-center justify-center w-7 h-7 rounded-md bg-white text-bg transition-colors hover:bg-white/80 disabled:opacity-30 disabled:cursor-not-allowed"
						>
							{submitPrompt.isPending ? (
								<Loader className="text-bg" />
							) : (
								<ArrowUp size={14} strokeWidth={2.5} />
							)}
						</button>
					</div>
				</div>
				{isRunning && ready ? (
					<div className="flex items-center gap-3 mt-3">
						<button
							type="button"
							disabled={createTerminal.isPending}
							onClick={() => createTerminal.mutate()}
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							{createTerminal.isPending ? <Loader /> : <Terminal size={12} />}
							Open Terminal
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							<Globe size={12} />
							Open Browser
						</button>
					</div>
				) : (
					<div className="flex items-center gap-2 mt-3 px-2 py-1 text-[11px] text-text-muted">
						<Loader className="text-text-muted" />
						<span>
							{isRunning ? "Waiting for SSH..." : statusMessage(status)}
						</span>
					</div>
				)}
			</div>
		</div>
	);
}
