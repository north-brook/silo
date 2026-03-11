"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { SiloIcon } from "./icons/silo";
import { TopBar } from "./components/top-bar";
import { toast } from "./hooks/use-toast";

export default function HomePage() {
	const queryClient = useQueryClient();
	const addProject = useMutation({
		mutationFn: async () => {
			const selected = await open({ directory: true, multiple: false });
			if (!selected) throw new Error("No folder selected");

			const path = typeof selected === "string" ? selected : selected;
			const name = path.split("/").pop() || path;

			await invoke("projects_add_project", { name, path });
			return name;
		},
		onSuccess: (name) => {
			queryClient.invalidateQueries({ queryKey: ["projects_list_projects"] });
			toast({ variant: "success", title: `Added project ${name}` });
		},
		onError: (error) => {
			if (error.message === "No folder selected") return;
			toast({ variant: "error", title: "Failed to add project", description: error.message });
		},
	});

	return (
		<div className="flex flex-col items-center justify-center h-full gap-6">
			<TopBar />
			<SiloIcon height={32} color="#FFFFFF" />
			<button
				type="button"
				onClick={() => addProject.mutate()}
				disabled={addProject.isPending}
				className="flex items-center gap-2 px-4 py-2 text-sm rounded-lg bg-btn border border-border-light text-text-bright hover:bg-btn-hover hover:border-border-hover transition-colors disabled:opacity-50"
			>
				<FolderOpen size={16} />
				Open Project
			</button>
		</div>
	);
}
