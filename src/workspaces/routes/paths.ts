import type { CloudSessionKind } from "@/workspaces/hosts/model";

export interface WorkspaceRouteState {
	fresh?: boolean;
	transition?: "resuming" | "saving";
}

export interface SessionRouteState extends WorkspaceRouteState {}

function encodePathSegment(value: string): string {
	return encodeURIComponent(value);
}

export function workspaceHref({
	project,
	workspace,
}: {
	project: string;
	workspace: string;
}): string {
	return `/projects/${encodePathSegment(project)}/workspaces/${encodePathSegment(workspace)}`;
}

export function browserSessionHref({
	project,
	workspace,
	attachmentId,
}: {
	project: string;
	workspace: string;
	attachmentId: string;
}): string {
	return `${workspaceHref({ project, workspace })}/browser/${encodePathSegment(attachmentId)}`;
}

export function terminalSessionHref({
	project,
	workspace,
	attachmentId,
}: {
	project: string;
	workspace: string;
	attachmentId: string;
}): string {
	return `${workspaceHref({ project, workspace })}/terminal/${encodePathSegment(attachmentId)}`;
}

export function fileSessionHref({
	project,
	workspace,
	attachmentId,
}: {
	project: string;
	workspace: string;
	attachmentId: string;
}): string {
	return `${workspaceHref({ project, workspace })}/file/${encodePathSegment(attachmentId)}`;
}

export function workspaceSessionHref({
	project,
	workspace,
	kind,
	attachmentId,
}: {
	project: string;
	workspace: string;
	kind: CloudSessionKind;
	attachmentId: string;
}): string {
	if (kind === "browser") {
		return browserSessionHref({ project, workspace, attachmentId });
	}

	if (kind === "file") {
		return fileSessionHref({ project, workspace, attachmentId });
	}

	return terminalSessionHref({ project, workspace, attachmentId });
}
