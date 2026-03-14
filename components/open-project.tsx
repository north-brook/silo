"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useRouter } from "next/navigation";
import { createContext, useContext, useEffect } from "react";
import { invoke } from "../../lib/invoke";
import type { Workspace } from "../../lib/workspaces";
import { toast } from "./toaster";

const OpenProjectContext = createContext<{
	open: () => void;
	isPending: boolean;
}>({
	open: () => {},
	isPending: false,
});

export const useOpenProject = () => useContext(OpenProjectContext);

export function OpenProjectProvider({
	children,
}: { children: React.ReactNode }) {
	const router = useRouter();
	const queryClient = useQueryClient();

	const addProject = useMutation({
		mutationFn: async () => {
			const selected = await open({ directory: true, multiple: false });
			if (!selected) throw new Error("No folder selected");

			const path = typeof selected === "string" ? selected : selected;
			const name = path.split("/").pop() || path;

			await invoke("projects_add_project", { name, path });
			const workspace = await invoke<Workspace>(
				"templates_create_template",
				{ project: name },
			);
			return { name, workspace };
		},
		onSuccess: ({ name, workspace }) => {
			queryClient.invalidateQueries({ queryKey: ["projects_list_projects"] });
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			router.push(
				`/workspace?project=${encodeURIComponent(name)}&name=${encodeURIComponent(workspace.name)}`,
			);
		},
		onError: (error) => {
			if (error.message === "No folder selected") return;
			toast({
				variant: "error",
				title: "Failed to open project",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		const handler = (e: KeyboardEvent) => {
			if (e.metaKey && e.shiftKey && e.key === "o") {
				e.preventDefault();
				addProject.mutate();
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [addProject]);

	return (
		<OpenProjectContext.Provider
			value={{
				open: () => addProject.mutate(),
				isPending: addProject.isPending,
			}}
		>
			{children}
		</OpenProjectContext.Provider>
	);
}
