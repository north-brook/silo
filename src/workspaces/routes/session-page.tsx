"use client";

import { useEffect, useMemo } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { BrowserSessionView } from "@/workspaces/browser/view";
import type { CloudSession } from "@/workspaces/hosts/model";
import { useWorkspaceState } from "@/workspaces/state";
import { TerminalSessionView } from "@/workspaces/terminal/view";
import { invoke } from "@/shared/lib/invoke";

export default function WorkspaceSessionPage() {
	return <WorkspaceSessionView />;
}

function WorkspaceSessionView() {
	const navigate = useNavigate();
	const [searchParams] = useSearchParams();
	const { cloudSessions, invalidateWorkspace, sessions } = useWorkspaceState();
	const workspace = searchParams.get("workspace") ?? "";
	const attachmentId = searchParams.get("attachment_id") ?? "";
	const kind = searchParams.get("kind") ?? "";
	const fresh = searchParams.get("fresh") === "1";
	const cleanParams = new URLSearchParams(searchParams.toString());
	cleanParams.delete("fresh");
	const cleanUrl = `/workspace/session?${cleanParams.toString()}`;

	useEffect(() => {
		if (!fresh) {
			return;
		}
		navigate(cleanUrl, { replace: true });
	}, [cleanUrl, fresh, navigate]);

	const hasLiveSession = useMemo(
		() =>
			sessions.some(
				(session) =>
					session.type === kind && session.attachment_id === attachmentId,
			),
		[attachmentId, kind, sessions],
	);
	const activeSession = useMemo<CloudSession | null>(() => {
		if (!workspace || !attachmentId || !kind) {
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
		if (!workspace || !kind || !attachmentId || !hasLiveSession) {
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

	if (!workspace || !attachmentId || !kind || !activeSession) {
		return null;
	}

	if (kind === "terminal") {
		return (
			<TerminalSessionView
				session={activeSession}
				skipInitialScrollback={fresh}
			/>
		);
	}

	if (kind !== "browser") {
		return (
			<div className="flex-1 min-h-0 bg-surface flex items-center justify-center p-6">
				<div className="text-[11px] text-text-muted">
					Unsupported session type: {kind}
				</div>
			</div>
		);
	}

	return (
		<BrowserSessionView
			session={activeSession}
			onChanged={invalidateWorkspace}
		/>
	);
}
