import { invoke } from "@/shared/lib/invoke";
import type { TemplateWorkspace } from "@/workspaces/api";

export interface ListedProject {
	name: string;
	path: string;
	image: string | null;
}

export interface SnapshotTemplate {
	name: string;
	project: string;
	created_at: string;
	status: string;
}

export type TemplateOperationKind =
	| "create"
	| "edit"
	| "save"
	| "delete";

export type TemplateOperationStatus = "running" | "completed" | "failed";

export interface TemplateOperation {
	project: string;
	workspace_name: string;
	kind: TemplateOperationKind;
	status: TemplateOperationStatus;
	phase: string;
	detail?: string | null;
	last_error?: string | null;
	snapshot_name?: string | null;
	updated_at: string;
}

export interface TemplateState {
	project: string;
	workspace_name: string;
	workspace_present: boolean;
	snapshot_name?: string | null;
	operation?: TemplateOperation | null;
}

export function listProjects(): Promise<ListedProject[]> {
	return invoke<ListedProject[]>("projects_list_projects");
}

export function listTemplates(): Promise<SnapshotTemplate[]> {
	return invoke<SnapshotTemplate[]>("templates_list_templates");
}

export function getTemplateState(project: string): Promise<TemplateState> {
	return invoke<TemplateState>("templates_get_state", { project });
}

export function createTemplate(project: string): Promise<TemplateWorkspace> {
	return invoke<TemplateWorkspace>("templates_create_template", { project });
}

export function editTemplate(project: string): Promise<TemplateWorkspace> {
	return invoke<TemplateWorkspace>("templates_edit_template", { project });
}

export function saveTemplate(project: string): Promise<TemplateOperation> {
	return invoke<TemplateOperation>("templates_save_template", { project });
}

export function deleteTemplate(project: string): Promise<TemplateOperation> {
	return invoke<TemplateOperation>("templates_delete_template", { project });
}
