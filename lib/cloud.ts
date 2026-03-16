"use client";

import type { WorkspaceSession } from "./workspaces";

export type CloudSessionKind = "terminal" | "browser" | "desktop" | string;

export interface CloudSession {
	workspace: string;
	kind: CloudSessionKind;
	attachmentId: string;
	name: string;
	working: boolean | null;
	unread: boolean | null;
}

export function normalizeTerminalSession(
	workspace: string,
	session: WorkspaceSession,
): CloudSession {
	return {
		workspace,
		kind: "terminal",
		attachmentId: session.attachment_id,
		name: session.name,
		working: session.working,
		unread: session.unread,
	};
}

export function cloudSessionKey(session: Pick<CloudSession, "workspace" | "kind" | "attachmentId">): string {
	return `${session.workspace}:${session.kind}:${session.attachmentId}`;
}

export function cloudSessionHref({
	project,
	workspace,
	kind,
	attachmentId,
	fresh,
}: {
	project: string;
	workspace: string;
	kind: CloudSessionKind;
	attachmentId: string;
	fresh?: boolean;
}): string {
	const params = new URLSearchParams({
		project,
		workspace,
		kind,
		attachment_id: attachmentId,
	});

	if (fresh) {
		params.set("fresh", "1");
	}

	return `/workspace/session?${params.toString()}`;
}
