"use client";

import { useRouter } from "next/navigation";
import { Suspense, useEffect, useMemo } from "react";
import { Loader } from "../../components/loader";
import { useWorkspaceState } from "../../components/workspace-state";
import { cloudSessionHref } from "../../lib/cloud";
import { isTemplateWorkspace, workspaceSessions } from "../../lib/workspaces";
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
	const { project, workspace } = useWorkspaceState();

	const redirectHref = useMemo(() => {
		if (!workspace || isTemplateWorkspace(workspace)) {
			return null;
		}

		const sessions = workspaceSessions(workspace);
		const activeSession = workspace.active_session
			? sessions.find(
					(session) =>
						session.type === workspace.active_session?.type &&
						session.attachment_id ===
							workspace.active_session?.attachment_id,
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
			workspace: workspace.name,
			kind: targetSession.type,
			attachmentId: targetSession.attachment_id,
		});
	}, [project, workspace]);

	useEffect(() => {
		if (!redirectHref) {
			return;
		}
		router.replace(redirectHref);
	}, [redirectHref, router]);

	if (!workspace) {
		return (
			<div className="flex-1 flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (redirectHref) {
		return null;
	}

	const isRunning = workspace.status === "RUNNING";

	if (isTemplateWorkspace(workspace)) {
		return (
			<TemplatingWorkspace
				isRunning={isRunning}
				ready={workspace.ready}
				status={workspace.status}
				workspace={workspace.name}
				project={workspace.project}
			/>
		);
	}

	return (
		<PromptWorkspace
			isRunning={isRunning}
			ready={workspace.ready}
			status={workspace.status}
			workspace={workspace.name}
			project={workspace.project}
		/>
	);
}
