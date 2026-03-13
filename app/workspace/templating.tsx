"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Laptop, Terminal } from "lucide-react";
import { useRouter } from "next/navigation";
import { invoke } from "../../lib/invoke";
import { Loader } from "../components/loader";
import { toast } from "../components/toaster";
import { SiloIcon } from "../icons/silo";

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

export function TemplatingWorkspace({
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
	const queryClient = useQueryClient();

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ terminal: string }>("terminal_create_terminal", {
				workspace,
			}),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			router.push(
				`/workspace/terminal?project=${encodeURIComponent(project ?? "")}&workspace=${encodeURIComponent(workspace)}&terminal=${encodeURIComponent(result.terminal)}`,
			);
		},
		onError: (error) => {
			toast({ variant: "error", title: "Failed to create terminal", description: error.message });
		},
	});

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="flex flex-col items-center gap-5">
				<SiloIcon height={24} className="opacity-40" />

				{isRunning ? (
					<div className="flex items-center gap-3">
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
							<Laptop size={12} />
							Open Desktop
						</button>
					</div>
				) : (
					<div className="flex items-center gap-2 px-2 py-1 text-[11px] text-text-muted">
						<Loader className="text-text-muted" />
						<span>{statusMessage(status)}</span>
					</div>
				)}
			</div>
		</div>
	);
}
