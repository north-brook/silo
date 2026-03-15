"use client";

import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import { Suspense } from "react";
import { invoke } from "../../lib/invoke";
import { isTemplateWorkspace, type Workspace } from "../../lib/workspaces";
import { SiloIcon } from "../../components/icons/silo";
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
					<div className="flex justify-center mb-6">
						<SiloIcon height={32} />
					</div>
					<div className="rounded-lg border border-border-light bg-surface overflow-hidden">
						<div className="px-4 pt-4 pb-2 min-h-[6rem]" />
						<div className="flex items-center justify-between px-3 pb-3">
							<div className="h-5 w-16 rounded bg-border-light/50 animate-pulse" />
							<div className="w-7 h-7 rounded-md bg-border-light/50 animate-pulse" />
						</div>
					</div>
				</div>
			</div>
		);
	}

	const isRunning = workspace.data.status === "RUNNING";

	if (isTemplateWorkspace(workspace.data)) {
		return (
			<TemplatingWorkspace
				isRunning={isRunning}
				ready={workspace.data.ready}
				status={workspace.data.status}
				workspace={workspace.data.name}
				project={workspace.data.project}
			/>
		);
	}

	return (
		<PromptWorkspace
			isRunning={isRunning}
			ready={workspace.data.ready}
			status={workspace.data.status}
			workspace={workspace.data.name}
			project={workspace.data.project}
		/>
	);
}
