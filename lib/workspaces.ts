import { invoke } from "./invoke";

export interface WorkspaceBase {
	name: string;
	project: string | null;
	last_active: string | null;
	created_at: string;
	status: string;
	zone: string;
	ready: boolean;
}

export interface BranchWorkspace extends WorkspaceBase {
	branch: string;
	target_branch: string;
	unread: boolean;
	working: boolean | null;
}

export interface TemplateWorkspace extends WorkspaceBase {
	template: true;
}

export type Workspace = TemplateWorkspace | BranchWorkspace;

export function isTemplateWorkspace(
	workspace: Workspace,
): workspace is TemplateWorkspace {
	return "template" in workspace;
}

export function workspaceLabel(workspace: Workspace): string {
	if (isTemplateWorkspace(workspace)) {
		return "template";
	}

	return workspace.branch || workspace.name;
}

export function createWorkspace(project: string): Promise<Workspace> {
	return invoke<Workspace>("workspaces_create_workspace", { project });
}
