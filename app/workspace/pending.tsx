"use client";

import { useMutation } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { Laptop, Terminal } from "lucide-react";
import { TerminalLoader } from "../components/terminal-loader";
import { invoke } from "../../lib/invoke";

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

export function PendingWorkspace({
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
				{isRunning ? (
					<div className="flex items-center gap-3">
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
					<div className="flex items-center gap-2 px-2 py-1 text-[11px] text-text-muted">
						<TerminalLoader className="text-text-muted" />
						<span>{statusMessage(status)}</span>
					</div>
				)}
			</div>
		</div>
	);
}
