"use client";

import { invoke } from "./invoke";
import type { TemplateWorkspace } from "./workspaces";

export interface SnapshotTemplate {
	name: string;
	project: string;
	created_at: string;
	status: string;
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
