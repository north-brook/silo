"use client";

import { useQuery } from "@tanstack/react-query";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo } from "react";
import { Loader } from "../../components/loader";
import { cloudSessionHref } from "../../lib/cloud";
import { invoke } from "../../lib/invoke";
import {
	isTemplateWorkspace,
	type Workspace,
	workspaceSessions,
} from "../../lib/workspaces";
import { PromptWorkspace } from "./prompt";
import { TemplatingWorkspace } from "./templating";

export default function WorkspacePage() {
	return (
		<Suspense>
			<WorkspaceView />
		</Suspense>
	);
}

function WorkspaceView() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const workspaceName = searchParams.get("name") ?? "";
	const projectParam = searchParams.get("project") ?? "";

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

	const redirectHref = useMemo(() => {
		if (!workspace.data || isTemplateWorkspace(workspace.data)) {
			return null;
		}

		const project = projectParam || workspace.data.project || "";
		const sessions = workspaceSessions(workspace.data);
		const activeSession = workspace.data.active_session
			? sessions.find(
					(session) =>
						session.type === workspace.data.active_session?.type &&
						session.attachment_id ===
							workspace.data.active_session?.attachment_id,
				)
			: null;
		const targetSession =
			activeSession ??
			(sessions.length > 0 ? sessions[sessions.length - 1] : null);
		if (!targetSession) {
			return null;
		}

		return cloudSessionHref({
			project,
			workspace: workspace.data.name,
			kind: targetSession.type,
			attachmentId: targetSession.attachment_id,
		});
	}, [projectParam, workspace.data]);

	useEffect(() => {
		if (!redirectHref) {
			return;
		}
		router.replace(redirectHref);
	}, [redirectHref, router]);

	if (!workspace.data) {
		return (
			<div className="flex-1 flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (redirectHref) {
		return null;
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
