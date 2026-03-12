import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useRouter, useSearchParams } from "next/navigation";
import {
	ChevronDown,
	Cpu,
	EllipsisVertical,
	Play,
	Plus,
	Square,
	Trash2,
} from "lucide-react";
import { Popover, PopoverTrigger, PopoverContent } from "./popover";
import { Tooltip, TooltipTrigger, TooltipContent } from "./tooltip";
import { toast } from "./toaster";
import Image from "next/image";
import { invoke } from "../../lib/invoke";
import type { Workspace } from "../../lib/workspaces";
import { workspaceLabel } from "../../lib/workspaces";
import { TerminalLoader } from "./terminal-loader";
import { WorkspaceIndicator } from "./workspace-status";

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
		// biome-ignore lint/a11y/useSemanticElements: can't use <button> because it contains interactive children
		<div
			role="button"
			tabIndex={0}
			onClick={onToggle}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					onToggle();
				}
			}}
			className="group flex items-center w-full px-3 py-2 text-xs text-text hover:bg-btn-hover hover:text-text-bright transition-colors cursor-pointer"
		>
			<span className="flex items-center gap-1.5 min-w-0 flex-1">
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
				<span className="truncate">{project.name}</span>
				<ChevronDown
					size={10}
					className={`shrink-0 text-text-placeholder transition-transform ${expanded ? "" : "-rotate-90"}`}
				/>
			</span>
			{isCreating ? (
				<span className="shrink-0 ml-auto p-1 -mr-1 flex items-center justify-center w-3 h-3 text-xs leading-none">
					<TerminalLoader className="text-text-muted" />
				</span>
			) : (
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={(e) => {
								e.stopPropagation();
								onCreate();
							}}
							className="shrink-0 ml-auto p-1 -mr-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity"
						>
							<Plus size={12} />
						</button>
					</TooltipTrigger>
					<TooltipContent side="right">New workspace</TooltipContent>
				</Tooltip>
			)}
		</div>
	);
}

function WorkspaceRow({ workspace }: { workspace: Workspace }) {
	const router = useRouter();
	const searchParams = useSearchParams();
	const queryClient = useQueryClient();
	const isActive = searchParams.get("name") === workspace.name;
	const isRunning = workspace.status === "RUNNING";
	const isStopped =
		workspace.status === "TERMINATED" || workspace.status === "STOPPED";

	const start = useMutation({
		mutationFn: () =>
			invoke("workspaces_start_workspace", { workspace: workspace.name }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace started" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to start",
				description: error.message,
			});
		},
	});

	const stop = useMutation({
		mutationFn: () =>
			invoke("workspaces_stop_workspace", { workspace: workspace.name }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace stopped" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to stop",
				description: error.message,
			});
		},
	});

	const remove = useMutation({
		mutationFn: () =>
			invoke("workspaces_delete_workspace", { workspace: workspace.name }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace deleted" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to delete",
				description: error.message,
			});
		},
	});

	const isStopping =
		workspace.status === "STOPPING" || workspace.status === "SUSPENDING";
	const isDisabled = stop.isPending || remove.isPending || isStopping;
	const [menuOpen, setMenuOpen] = useState(false);

	const navigate = () =>
		router.push(
			`/workspace?project=${encodeURIComponent(workspace.project ?? "")}&name=${encodeURIComponent(workspace.name)}`,
		);

	return (
		// biome-ignore lint/a11y/useSemanticElements: can't use <button> because it contains interactive children
		<div
			role="button"
			tabIndex={0}
			onClick={navigate}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					navigate();
				}
			}}
			className={`group flex items-center w-full pl-5 pr-3 py-1.5 text-xs transition-colors cursor-pointer ${
				isDisabled
					? "opacity-30 pointer-events-none"
					: isActive
						? "bg-btn-hover text-text-bright"
						: "text-text-muted hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			<span className="flex items-center gap-2 min-w-0 flex-1">
				<WorkspaceIndicator workspace={workspace} />
				<span className="truncate">{workspaceLabel(workspace)}</span>
			</span>
			<Popover open={menuOpen} onOpenChange={setMenuOpen}>
				<PopoverTrigger asChild>
					<button
						type="button"
						onClick={(e) => e.stopPropagation()}
						className="shrink-0 ml-auto p-1 -mr-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity"
					>
						<EllipsisVertical size={12} />
					</button>
				</PopoverTrigger>
				<PopoverContent side="right" align="start" className="w-36 p-1">
					{isStopped && (
						<button
							type="button"
							onClick={(e) => {
								e.stopPropagation();
								start.mutate();
								setMenuOpen(false);
							}}
							className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
						>
							<Play size={12} />
							Start
						</button>
					)}
					{isRunning && (
						<button
							type="button"
							onClick={(e) => {
								e.stopPropagation();
								setMenuOpen(false);
								router.push("/");
								stop.mutate();
							}}
							className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
						>
							<Square size={12} />
							Stop
						</button>
					)}
					<button
						type="button"
						onClick={(e) => {
							e.stopPropagation();
							setMenuOpen(false);
							router.push("/");
							remove.mutate();
						}}
						className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-error hover:bg-error/10 rounded transition-colors"
					>
						<Trash2 size={12} />
						Delete
					</button>
				</PopoverContent>
			</Popover>
		</div>
	);
}

function BarFooter() {
	const memory = useQuery({
		queryKey: ["system_memory_usage"],
		queryFn: () =>
			invoke<number>("system_memory_usage", {
				log: "state_changes_only",
				key: "poll:system_memory_usage",
				stateChanged: (previous, next) =>
					Math.round((previous ?? 0) / 50) !== Math.round(next / 50),
			}),
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
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});
	const workspaces = useQuery({
		queryKey: ["workspaces_list_workspaces"],
		queryFn: () =>
			invoke<Workspace[]>("workspaces_list_workspaces", {
				log: "state_changes_only",
				key: "poll:workspaces_list_workspaces",
			}),
		refetchInterval: 15000,
	});
	const [expanded, setExpanded] = useState<Record<string, boolean>>({});

	const createWorkspace = useMutation({
		mutationFn: (project: string) =>
			invoke<Workspace>("workspaces_create_workspace", { project }),
		onSuccess: (workspace, project) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace created" });
			router.push(
				`/workspace?project=${encodeURIComponent(project)}&name=${encodeURIComponent(workspace.name)}`,
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create workspace",
				description: error.message,
			});
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
				{projects.data.map((project) => {
					const projectWorkspaces = (workspaces.data ?? []).filter(
						(w) => w.project === project.name,
					);
					return (
						<div key={project.name}>
							<ProjectRow
								project={project}
								expanded={isExpanded(project.name)}
								onToggle={() => toggle(project.name)}
								onCreate={() => createWorkspace.mutate(project.name)}
								isCreating={createWorkspace.isPending}
							/>
							{isExpanded(project.name) &&
								projectWorkspaces.map((w) => (
									<WorkspaceRow key={w.name} workspace={w} />
								))}
						</div>
					);
				})}
			</div>
			<BarFooter />
		</aside>
	);
}
