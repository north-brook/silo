"use client";

import { useQuery } from "@tanstack/react-query";
import { Terminal } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { invoke } from "../../lib/invoke";
import type { Workspace } from "../../lib/workspaces";
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
			<header className="h-8 w-full border-b border-border-light shrink-0 flex items-center relative">
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

	const activeTerminal = searchParams.get("terminal");
	const project = searchParams.get("project") ?? workspace.data?.project ?? "";
	const terminalList = terminals.data ?? [];

	return (
		<>
			{workspace.data ? (
				<TopBar workspace={workspace.data} />
			) : (
				<header className="h-8 w-full border-b border-border-light shrink-0 flex items-center relative">
					<div data-tauri-drag-region className="absolute inset-0" />
					<div className="relative flex items-center gap-1.5 px-3 z-10">
						<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
						<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
					</div>
				</header>
			)}
			{terminalList.length > 0 && (
				<div className="h-7 w-full border-b border-border-light shrink-0 flex items-center gap-0 px-2 overflow-x-auto">
					{terminalList.map((session) => {
						const isActive = activeTerminal === session.name;
						return (
							<button
								key={session.name}
								type="button"
								onClick={() =>
									router.push(
										`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspaceName)}&terminal=${encodeURIComponent(session.name)}`,
									)
								}
								className={`h-full flex items-center gap-1.5 px-2.5 text-[11px] shrink-0 border-b-2 transition-colors ${
									isActive
										? "text-text-bright border-text-bright"
										: "text-text-muted border-transparent hover:text-text-bright"
								}`}
							>
								<Terminal size={12} />
								{session.name}
							</button>
						);
					})}
				</div>
			)}
			{children}
		</>
	);
}
