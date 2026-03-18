import { useEffect, useMemo, useRef } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { BrowserSessionView } from "@/workspaces/browser/view";
import type { CloudSession } from "@/workspaces/hosts/model";
import {
	useCloudSessions,
	useWorkspaceSessions,
	useWorkspaceState,
} from "@/workspaces/state";
import { TerminalSessionView } from "@/workspaces/terminal/view";
import { invoke } from "@/shared/lib/invoke";
import {
	type SessionRouteState,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";
import { useWorkspaceSessionRouteParams } from "@/workspaces/routes/params";

export function WorkspaceBrowserSessionPage() {
	return <WorkspaceSessionView kind="browser" />;
}

export function WorkspaceTerminalSessionPage() {
	return <WorkspaceSessionView kind="terminal" />;
}

function WorkspaceSessionView({ kind }: { kind: "browser" | "terminal" }) {
	const location = useLocation();
	const navigate = useNavigate();
	const freshRef = useRef(
		(location.state as SessionRouteState | null)?.fresh === true,
	);
	const { invalidateWorkspace } = useWorkspaceState();
	const sessions = useWorkspaceSessions();
	const cloudSessions = useCloudSessions();
	const {
		attachmentId,
		project,
		workspaceName: workspace,
	} = useWorkspaceSessionRouteParams();

	useEffect(() => {
		if (!freshRef.current) {
			return;
		}

		navigate(
			workspaceSessionHref({
				project,
				workspace,
				kind,
				attachmentId,
			}),
			{ replace: true, state: null },
		);
	}, [attachmentId, kind, navigate, project, workspace]);

	const hasLiveSession = useMemo(
		() =>
			sessions.some(
				(session) =>
					session.type === kind && session.attachment_id === attachmentId,
			),
		[attachmentId, kind, sessions],
	);

	const activeSession = useMemo<CloudSession | null>(() => {
		if (!workspace || !attachmentId) {
			return null;
		}

		return (
			cloudSessions.find(
				(session) =>
					session.kind === kind && session.attachmentId === attachmentId,
			) ?? {
				workspace,
				kind,
				attachmentId,
				name: attachmentId,
				url: null,
				logicalUrl: null,
				resolvedUrl: null,
				title: null,
				faviconUrl: null,
				canGoBack: null,
				canGoForward: null,
				working: null,
				unread: null,
			}
		);
	}, [attachmentId, cloudSessions, kind, workspace]);

	useEffect(() => {
		if (!workspace || !attachmentId || !hasLiveSession) {
			return;
		}

		const timeout = window.setTimeout(() => {
			void invoke("workspaces_set_active_session", {
				workspace,
				kind,
				attachmentId,
			});
		}, 200);

		return () => {
			window.clearTimeout(timeout);
		};
	}, [attachmentId, hasLiveSession, kind, workspace]);

	if (!workspace || !attachmentId || !activeSession) {
		return null;
	}

	if (kind === "terminal") {
		return (
			<TerminalSessionView
				session={activeSession}
				skipInitialScrollback={freshRef.current}
			/>
		);
	}

	return (
		<BrowserSessionView
			session={activeSession}
			onChanged={invalidateWorkspace}
		/>
	);
}
