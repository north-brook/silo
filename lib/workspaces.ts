export interface WorkspaceBase {
	name: string;
	project: string | null;
	last_active: string | null;
	created_at: string;
	status: string;
	zone: string;
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
		return workspace.name;
	}

	return workspace.branch || workspace.name;
}
