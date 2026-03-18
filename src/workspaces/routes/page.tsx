"use client";

import { useEffect, useMemo } from "react";
import { useNavigate } from "react-router-dom";
import { Loader } from "@/shared/ui/loader";
import { useWorkspaceState } from "@/workspaces/state";
import { cloudSessionHref } from "@/workspaces/routes/paths";
import { isTemplateWorkspace, workspaceSessions } from "@/workspaces/api";
import { PromptWorkspace } from "@/workspaces/prompt/screen";
import { TemplatingWorkspace } from "@/workspaces/template/screen";

export default function WorkspacePage() {
	return <WorkspaceView />;
}

function WorkspaceView() {
	const navigate = useNavigate();
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
						session.attachment_id === workspace.active_session?.attachment_id,
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
		navigate(redirectHref, { replace: true });
	}, [navigate, redirectHref]);

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
