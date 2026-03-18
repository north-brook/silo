"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import { createContext, useContext, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@/shared/lib/invoke";
import type { ListedProject, SnapshotTemplate } from "@/projects/api";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import { createWorkspace as createWorkspaceCommand } from "@/workspaces/api";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "@/shared/ui/dialog";
import Image from "@/shared/ui/image";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";

const NewWorkspaceContext = createContext<{ open: () => void }>({
	open: () => {},
});

export const useNewWorkspace = () => useContext(NewWorkspaceContext);

export function NewWorkspaceProvider({
	children,
}: {
	children: React.ReactNode;
}) {
	const [isOpen, setIsOpen] = useState(false);

	const projects = useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});

	const templates = useQuery({
		queryKey: ["templates_list_templates"],
		queryFn: () =>
			invoke<SnapshotTemplate[]>("templates_list_templates", {
				log: "state_changes_only",
				key: "poll:templates_list_templates",
				stateChanged: (previous: unknown, next: unknown) =>
					JSON.stringify(previous) !== JSON.stringify(next),
			}),
		refetchInterval: 15000,
	});

	useShortcut<void>({
		event: shortcutEvents.newWorkspace,
		onTrigger: () => {
			setIsOpen(true);
		},
		onKeyDown: (e) => {
			if (e.metaKey && e.key === "n") {
				e.preventDefault();
				setIsOpen(true);
			}
		},
	});

	return (
		<NewWorkspaceContext.Provider value={{ open: () => setIsOpen(true) }}>
			{children}
			<NewWorkspaceDialog
				open={isOpen}
				onOpenChange={setIsOpen}
				projects={projects.data ?? []}
				templates={templates.data ?? []}
			/>
		</NewWorkspaceContext.Provider>
	);
}

function NewWorkspaceDialog({
	open,
	onOpenChange,
	projects,
	templates,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	projects: ListedProject[];
	templates: SnapshotTemplate[];
}) {
	const navigate = useNavigate();
	const queryClient = useQueryClient();

	const projectsWithTemplates = projects.filter((p) =>
		templates.some((t) => t.project === p.name),
	);

	const createWorkspace = useMutation({
		mutationFn: (projectName: string) => createWorkspaceCommand(projectName),
		onSuccess: (workspace) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace created" });
			navigate(
				`/workspace?project=${encodeURIComponent(workspace.project ?? "")}&name=${encodeURIComponent(workspace.name)}`,
			);
			onOpenChange(false);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create workspace",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		if (!open) return;
		const handler = (e: KeyboardEvent) => {
			const num = Number.parseInt(e.key, 10);
			if (num >= 1 && num <= projectsWithTemplates.length) {
				e.preventDefault();
				createWorkspace.mutate(projectsWithTemplates[num - 1].name);
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [open, projectsWithTemplates, createWorkspace]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				onOpenAutoFocus={(e) => e.preventDefault()}
				className="max-w-xs p-0 gap-0"
			>
				<DialogHeader className="p-4 pb-2">
					<DialogTitle>New Workspace</DialogTitle>
				</DialogHeader>
				<div className="flex flex-col pt-1 pb-3">
					{projectsWithTemplates.map((project, index) => (
						<button
							key={project.name}
							type="button"
							disabled={createWorkspace.isPending}
							onClick={() => createWorkspace.mutate(project.name)}
							className="flex items-center gap-2.5 w-full px-4 py-2 text-xs text-text hover:bg-btn-hover hover:text-text-bright transition-colors disabled:opacity-50"
						>
							{project.image ? (
								<Image
									width={16}
									height={16}
									src={convertFileSrc(project.image)}
									alt={project.name}
									className="rounded object-cover shrink-0"
								/>
							) : (
								<div className="w-4 h-4 rounded bg-border-light shrink-0" />
							)}
							<span className="truncate flex-1 text-left">{project.name}</span>
							{createWorkspace.isPending &&
							createWorkspace.variables === project.name ? (
								<Loader className="text-text-muted" />
							) : (
								<kbd className="shrink-0 text-[10px] text-text-placeholder border border-border-light rounded px-1.5 py-0.5">
									{index + 1}
								</kbd>
							)}
						</button>
					))}
					{projectsWithTemplates.length === 0 && (
						<p className="px-4 py-3 text-xs text-text-muted">
							No projects with templates yet.
						</p>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}
