"use client";

import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { invoke } from "../../lib/invoke";
import { isTemplateWorkspace, type Workspace } from "../../lib/workspaces";
import { TemplatingWorkspace } from "./templating";
import { PromptWorkspace } from "./prompt";

export default function WorkspacePage() {
	return (
		<Suspense>
			<WorkspaceView />
		</Suspense>
	);
}

function WorkspaceView() {
	const searchParams = useSearchParams();
	const workspaceName = searchParams.get("name") ?? "";

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
		refetchInterval: 2000,
	});

	if (!workspace.data) {
		return (
			<div className="flex-1 flex flex-col items-center justify-center p-6">
				<div className="w-full max-w-2xl">
					<div className="h-40 rounded-lg bg-border-light/50 animate-pulse" />
				</div>
			</div>
		);
	}

	const isRunning = workspace.data.status === "RUNNING";

	if (isTemplateWorkspace(workspace.data)) {
		return (
			<TemplatingWorkspace
				isRunning={isRunning}
				status={workspace.data.status}
				workspace={workspace.data.name}
				project={workspace.data.project}
			/>
		);
	}

	const isPrompt = !workspace.data.last_active;

	return isPrompt ? (
		<PromptWorkspace
			isRunning={isRunning}
			status={workspace.data.status}
			workspace={workspace.data.name}
			project={workspace.data.project}
		/>
	) : (
		<div className="flex-1 overflow-auto p-4" />
	);
}
