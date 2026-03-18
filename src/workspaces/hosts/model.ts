import type { WorkspaceSession } from "@/workspaces/api";

export type CloudSessionKind = "terminal" | "browser" | "desktop" | string;

export interface CloudSession {
	workspace: string;
	kind: CloudSessionKind;
	attachmentId: string;
	name: string;
	url: string | null;
	logicalUrl: string | null;
	resolvedUrl: string | null;
	title: string | null;
	faviconUrl: string | null;
	canGoBack: boolean | null;
	canGoForward: boolean | null;
	working: boolean | null;
	unread: boolean | null;
}

export function normalizeWorkspaceSession(
	workspace: string,
	session: WorkspaceSession,
): CloudSession {
	return {
		workspace,
		kind: session.type,
		attachmentId: session.attachment_id,
		name: session.name,
		url: session.logical_url ?? session.url ?? null,
		logicalUrl: session.logical_url ?? session.url ?? null,
		resolvedUrl: session.resolved_url ?? null,
		title: session.title ?? null,
		faviconUrl: session.favicon_url ?? null,
		canGoBack: session.can_go_back ?? null,
		canGoForward: session.can_go_forward ?? null,
		working: session.working,
		unread: session.unread,
	};
}

export function cloudSessionKey(
	session: Pick<CloudSession, "workspace" | "kind" | "attachmentId">,
): string {
	return `${session.workspace}:${session.kind}:${session.attachmentId}`;
}
