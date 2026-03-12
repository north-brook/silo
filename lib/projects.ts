"use client";

import { invoke } from "./invoke";

export interface ListedProject {
	name: string;
	path: string;
	image: string | null;
}

export function listProjects(): Promise<ListedProject[]> {
	return invoke<ListedProject[]>("projects_list_projects");
}
