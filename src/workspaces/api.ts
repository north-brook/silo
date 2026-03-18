import { invoke } from "@/shared/lib/invoke";

export interface WorkspaceBase {
	name: string;
	project: string | null;
	last_active: string | null;
	active_session?: WorkspaceActiveSession | null;
	created_at: string;
	status: string;
	zone: string;
	ready: boolean;
}

export interface WorkspaceActiveSession {
	type: string;
	attachment_id: string;
}

export interface WorkspaceSession {
	type: string;
	name: string;
	attachment_id: string;
	url?: string | null;
	logical_url?: string | null;
	resolved_url?: string | null;
	title?: string | null;
	favicon_url?: string | null;
	can_go_back?: boolean | null;
	can_go_forward?: boolean | null;
	working: boolean | null;
	unread: boolean | null;
}

export interface WorkspacePromptResult {
	attachment_id: string;
}

export interface BranchWorkspace extends WorkspaceBase {
	branch: string;
	target_branch: string;
	unread: boolean;
	working: boolean | null;
	terminals: WorkspaceSession[];
	browsers: WorkspaceSession[];
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

export function workspaceSessions(workspace: Workspace): WorkspaceSession[] {
	if (isTemplateWorkspace(workspace)) {
		return [];
	}

	return [...workspace.terminals, ...workspace.browsers].sort((left, right) => {
		const leftTimestamp = sessionTimestamp(left.attachment_id);
		const rightTimestamp = sessionTimestamp(right.attachment_id);
		if (leftTimestamp !== rightTimestamp) {
			return leftTimestamp - rightTimestamp;
		}
		return left.attachment_id.localeCompare(right.attachment_id);
	});
}

function sessionTimestamp(attachmentId: string): number {
	const [, rawTimestamp = "0"] = attachmentId.split("-", 2);
	const timestamp = Number.parseInt(rawTimestamp, 10);
	return Number.isFinite(timestamp) ? timestamp : 0;
}

export function createWorkspace(project: string): Promise<Workspace> {
	return invoke<Workspace>("workspaces_create_workspace", { project });
}

export function submitWorkspacePrompt(
	workspace: string,
	prompt: string,
	model: "codex" | "claude",
): Promise<WorkspacePromptResult> {
	return invoke<WorkspacePromptResult>("workspaces_submit_prompt", {
		workspace,
		prompt,
		model,
	});
}
