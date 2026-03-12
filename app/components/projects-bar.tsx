import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useRouter } from "next/navigation";
import { ChevronDown, Cpu, Plus } from "lucide-react";
import { Tooltip, TooltipTrigger, TooltipContent } from "./tooltip";
import { toast } from "./toaster";
import Image from "next/image";
import { invokeLogged } from "../../lib/logging";

interface Workspace {
	name: string;
}

interface ListedProject {
	name: string;
	path: string;
	image: string | null;
}

function ProjectRow({
	project,
	expanded,
	onToggle,
	onCreate,
	isCreating,
}: {
	project: ListedProject;
	expanded: boolean;
	onToggle: () => void;
	onCreate: () => void;
	isCreating: boolean;
}) {
	return (
		<div className="group flex items-center w-full px-3 py-2 text-xs text-text hover:bg-btn-hover hover:text-text-bright transition-colors">
			<button
				type="button"
				onClick={onToggle}
				className="flex items-center gap-2.5 min-w-0 flex-1"
			>
				{project.image ? (
					<Image
						width={20}
						height={20}
						src={convertFileSrc(project.image)}
						alt={project.name}
						className="rounded object-cover shrink-0"
					/>
				) : (
					<div className="w-5 h-5 rounded bg-border-light shrink-0" />
				)}
				<span className="truncate">{project.name}</span>
				<ChevronDown
					size={10}
					className={`shrink-0 text-text-placeholder transition-transform ${expanded ? "" : "-rotate-90"}`}
				/>
			</button>
			<Tooltip>
				<TooltipTrigger asChild>
					<button
						type="button"
						onClick={onCreate}
						disabled={isCreating}
						className="shrink-0 ml-auto p-1 -mr-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity disabled:opacity-50"
					>
						<Plus size={12} />
					</button>
				</TooltipTrigger>
				<TooltipContent side="right">New workspace</TooltipContent>
			</Tooltip>
		</div>
	);
}

function BarFooter() {
	const memory = useQuery({
		queryKey: ["system_memory_usage"],
		queryFn: () => invokeLogged<number>("system_memory_usage"),
		refetchInterval: 5000,
	});

	return (
		<div className="shrink-0 px-3 py-2 text-[11px] text-text-muted">
			{memory.data !== undefined && (
				<span className="flex items-center gap-1">
					<Cpu size={10} />
					{memory.data.toFixed(1)} MB
				</span>
			)}
		</div>
	);
}

export function ProjectsBar() {
	const router = useRouter();
	const queryClient = useQueryClient();
	const projects = useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invokeLogged<ListedProject[]>("projects_list_projects"),
	});
	const [expanded, setExpanded] = useState<Record<string, boolean>>({});

	const createWorkspace = useMutation({
		mutationFn: (project: string) =>
			invokeLogged<Workspace>("workspaces_create_workspace", { project }),
		onSuccess: (workspace, project) => {
			queryClient.invalidateQueries({ queryKey: ["workspaces_list_workspaces", project] });
			toast({ variant: "success", title: "Workspace created" });
			router.push(`/workspace?project=${encodeURIComponent(project)}&name=${encodeURIComponent(workspace.name)}`);
		},
		onError: (error) => {
			toast({ variant: "error", title: "Failed to create workspace", description: error.message });
		},
	});

	if (!projects.data || projects.data.length === 0) return null;

	const isExpanded = (name: string) => expanded[name] !== false;
	const toggle = (name: string) =>
		setExpanded((prev) => ({ ...prev, [name]: !isExpanded(name) }));

	return (
		<aside className="w-48 shrink-0 border-r border-border-light bg-surface flex flex-col">
			<div data-tauri-drag-region className="h-8 shrink-0" />
			<div className="flex-1 overflow-y-auto pb-1">
				{projects.data.map((project) => (
					<ProjectRow
						key={project.name}
						project={project}
						expanded={isExpanded(project.name)}
						onToggle={() => toggle(project.name)}
						onCreate={() => createWorkspace.mutate(project.name)}
						isCreating={createWorkspace.isPending}
					/>
				))}
			</div>
			<BarFooter />
		</aside>
	);
}
