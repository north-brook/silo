"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Terminal, X } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useState } from "react";
import { invoke } from "../../lib/invoke";
import type { Workspace } from "../../lib/workspaces";
import { Loader } from "../components/loader";
import { toast } from "../components/toaster";
import { TopBar } from "../components/top-bar";

interface TerminalSessionSummary {
	name: string;
	pid: number | null;
	clients: number;
	started_in: string | null;
	created_at: string | null;
}

export default function WorkspaceLayout({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	return (
		<Suspense fallback={<LayoutSkeleton>{children}</LayoutSkeleton>}>
			<WorkspaceLayoutInner>{children}</WorkspaceLayoutInner>
		</Suspense>
	);
}

function LayoutSkeleton({ children }: { children: React.ReactNode }) {
	return (
		<>
			<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
				<div data-tauri-drag-region className="absolute inset-0" />
				<div className="relative flex items-center gap-1.5 px-3 z-10">
					<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
					<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
				</div>
			</header>
			{children}
		</>
	);
}

function WorkspaceLayoutInner({ children }: { children: React.ReactNode }) {
	const searchParams = useSearchParams();
	const router = useRouter();
	const queryClient = useQueryClient();
	const workspaceName =
		searchParams.get("name") ?? searchParams.get("workspace") ?? "";

	const workspace = useQuery({
		queryKey: ["workspaces_get_workspace", workspaceName],
		queryFn: () =>
			invoke<Workspace>(
				"workspaces_get_workspace",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:workspaces_get_workspace:${workspaceName}`,
				},
			),
		enabled: !!workspaceName,
		refetchInterval: 10000,
	});

	const terminals = useQuery({
		queryKey: ["terminal_list_terminals", workspaceName],
		queryFn: () =>
			invoke<TerminalSessionSummary[]>(
				"terminal_list_terminals",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:terminal_list_terminals:${workspaceName}`,
				},
			),
		enabled: !!workspaceName && workspace.data?.status === "RUNNING",
		refetchInterval: 5000,
	});

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ terminal: string }>("terminal_create_terminal", {
				workspace: workspaceName,
			}),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspaceName],
			});
			router.push(
				`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspaceName)}&terminal=${encodeURIComponent(result.terminal)}`,
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

	const activeTerminal = searchParams.get("terminal");
	const project = searchParams.get("project") ?? workspace.data?.project ?? "";
	const terminalList = terminals.data ?? [];

	const [killingTerminal, setKillingTerminal] = useState<string | null>(null);

	const killTerminal = useMutation({
		mutationFn: (name: string) =>
			invoke("terminal_kill_terminal", {
				workspace: workspaceName,
				name,
			}),
		onMutate: (name) => {
			setKillingTerminal(name);
		},
		onSettled: () => {
			setKillingTerminal(null);
		},
		onSuccess: (_result, name) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspaceName],
			});
			const index = terminalList.findIndex((s) => s.name === name);
			const leftNeighbor = index > 0 ? terminalList[index - 1] : null;
			if (activeTerminal === name && leftNeighbor) {
				router.push(
					`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspaceName)}&terminal=${encodeURIComponent(leftNeighbor.name)}`,
				);
			} else if (activeTerminal === name) {
				router.push(`/workspace?name=${encodeURIComponent(workspaceName)}`);
			}
		},
	});

	return (
		<>
			{workspace.data ? (
				<TopBar workspace={workspace.data} />
			) : (
				<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
					<div data-tauri-drag-region className="absolute inset-0" />
					<div className="relative flex items-center gap-1.5 px-3 z-10">
						<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
						<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
					</div>
				</header>
			)}
			{terminalList.length > 0 && (
				<div className="w-full bg-bg shrink-0 flex items-end overflow-x-auto">
					{terminalList.map((session) => {
						const isActive = activeTerminal === session.name;
						return (
							// biome-ignore lint/a11y/noStaticElementInteractions: can't use <button> because it contains interactive children
							<div
								key={session.name}
								onClick={() =>
									router.push(
										`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspaceName)}&terminal=${encodeURIComponent(session.name)}`,
									)
								}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										router.push(
											`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspaceName)}&terminal=${encodeURIComponent(session.name)}`,
										);
									}
								}}
								className={`group/tab h-9 flex items-center gap-1.5 pl-3 pr-2 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
									isActive
										? "bg-surface text-text-bright border-r-border-light border-b-surface"
										: "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
								}`}
							>
								<Terminal size={12} />
								Terminal
								{killingTerminal === session.name ? (
									<span className="p-0.5">
										<Loader />
									</span>
								) : (
									<button
										type="button"
										onClick={(e) => {
											e.stopPropagation();
											killTerminal.mutate(session.name);
										}}
										className={`p-0.5 rounded transition-colors hover:bg-border-light ${
											isActive
												? "text-text-muted hover:text-text-bright"
												: "opacity-0 group-hover/tab:opacity-100 text-text-muted hover:text-text-bright"
										}`}
									>
										<X size={10} />
									</button>
								)}
							</div>
						);
					})}
					<button
						type="button"
						disabled={createTerminal.isPending}
						onClick={() => createTerminal.mutate()}
						className="h-9 flex items-center px-2.5 border-b border-border-light text-text-muted hover:text-text-bright transition-colors disabled:opacity-50"
					>
						{createTerminal.isPending ? <Loader /> : <Plus size={12} />}
					</button>
					<div className="flex-1 h-9 border-b border-border-light" />
				</div>
			)}
			{children}
		</>
	);
}
