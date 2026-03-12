"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { TopBar } from "../components/top-bar";
import { PromptWorkspace } from "./prompt";
import { invoke } from "../../lib/invoke";

interface Workspace {
	name: string;
	project: string | null;
	branch: string;
	target_branch: string;
	unread: boolean;
	working: boolean | null;
	last_active: string | null;
	created_at: string;
	status: string;
	zone: string;
}

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

	const isRunning = workspace.data?.status === "RUNNING";
	const isPrompt = !workspace.data?.last_active;

	return (
		<>
			<TopBar
				workspace={workspaceName}
				project={project}
				branch={workspace.data?.branch ?? ""}
				targetBranch={workspace.data?.target_branch ?? ""}
			/>
			{isPrompt ? (
				<PromptWorkspace isRunning={isRunning} />
			) : (
				<div className="flex-1 overflow-auto p-4" />
			)}
		</>
	);
}
