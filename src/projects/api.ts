"use client";

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

export function listProjects(): Promise<ListedProject[]> {
	return invoke<ListedProject[]>("projects_list_projects");
}

export function listTemplates(): Promise<SnapshotTemplate[]> {
	return invoke<SnapshotTemplate[]>("templates_list_templates");
}

export function createTemplate(project: string): Promise<TemplateWorkspace> {
	return invoke<TemplateWorkspace>("templates_create_template", { project });
}

export function editTemplate(project: string): Promise<TemplateWorkspace> {
	return invoke<TemplateWorkspace>("templates_edit_template", { project });
}

export function saveTemplate(project: string): Promise<void> {
	return invoke<void>("templates_save_template", { project });
}

export function deleteTemplate(project: string): Promise<void> {
	return invoke<void>("templates_delete_template", { project });
}
