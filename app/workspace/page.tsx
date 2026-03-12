"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { TopBar } from "../components/top-bar";
import { PromptWorkspace } from "./prompt";
import { invoke } from "../../lib/invoke";
import type { Workspace } from "../../lib/workspaces";

export default function WorkspacePage() {
	return (
		<Suspense>
			<WorkspaceView />
		</Suspense>
	);
}

function WorkspaceLoading() {
	return (
		<>
			<header className="h-8 w-full border-b border-border-light shrink-0 flex items-center relative">
				<div data-tauri-drag-region className="absolute inset-0" />
				<div className="relative flex items-center gap-1.5 px-3 z-10">
					<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
					<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
				</div>
			</header>
			<div className="flex-1 flex flex-col items-center justify-center p-6">
				<div className="w-full max-w-2xl">
					<div className="h-40 rounded-lg bg-border-light/50 animate-pulse" />
				</div>
			</div>
		</>
	);
}

function WorkspaceView() {
	const searchParams = useSearchParams();
	const workspaceName = searchParams.get("name") ?? "";
	const project = searchParams.get("project") ?? "";

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

	if (!workspace.data) {
		return <WorkspaceLoading />;
	}

	const isRunning = workspace.data.status === "RUNNING";
	const isPrompt = !workspace.data.last_active;

	return (
		<>
			<TopBar
				workspace={workspaceName}
				project={project}
				workspaceData={workspace.data}
			/>
			{isPrompt ? (
				<PromptWorkspace isRunning={isRunning} status={workspace.data.status} />
			) : (
				<div className="flex-1 overflow-auto p-4" />
			)}
		</>
	);
}
