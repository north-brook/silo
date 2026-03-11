"use client";

import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

export interface ListedProject {
	name: string;
	path: string;
	image: string | null;
}

export function useProjects() {
	return useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});
}
