"use client";

import { useState } from "react";
import { ArrowUp } from "lucide-react";
import { Popover, PopoverTrigger, PopoverContent } from "../components/popover";
import { ClaudeIcon } from "../icons/claude";
import { CodexIcon } from "../icons/codex";

const PROVIDERS = [
	{
		id: "codex",
		label: "Codex",
		icon: <CodexIcon height={14} color="#FFFFFF" />,
	},
	{
		id: "claude",
		label: "Claude",
		icon: <ClaudeIcon height={14} color="#D97757" />,
	},
] as const;

export function PromptWorkspace({
	isRunning,
}: {
	isRunning: boolean;
}) {
	const [prompt, setPrompt] = useState("");
	const [provider, setProvider] = useState(PROVIDERS[0]);
	const [providerOpen, setProviderOpen] = useState(false);
	const canSubmit = isRunning && prompt.trim().length > 0;

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
				<div className="w-full max-w-2xl">
					<div className="rounded-lg border border-border-light bg-surface overflow-hidden">
						<textarea
							value={prompt}
							onChange={(e) => setPrompt(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter" && !e.shiftKey && canSubmit) {
									e.preventDefault();
								}
							}}
							placeholder="What do you want to do?"
							rows={4}
							className="w-full resize-none bg-transparent border-0 px-4 pt-4 pb-2 text-sm text-text-bright placeholder:text-text-placeholder outline-none focus:border-0 focus:ring-0"
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
				</div>
		</div>
	);
}
