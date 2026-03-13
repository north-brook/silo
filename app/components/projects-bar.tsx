import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
	ChevronDown,
	Cpu,
	EllipsisVertical,
	Play,
	Plus,
	Save,
	Box,
	Square,
	Trash2,
} from "lucide-react";
import Image from "next/image";
import { useRouter, useSearchParams } from "next/navigation";
import { useState } from "react";
import { invoke } from "../../lib/invoke";
import type { ListedProject } from "../../lib/projects";
import type { SnapshotTemplate } from "../../lib/templates";
import {
	createWorkspace as createWorkspaceCommand,
	isTemplateWorkspace,
	type Workspace,
	workspaceLabel,
} from "../../lib/workspaces";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { Loader } from "./loader";
import { toast } from "./toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";
import { WorkspaceIndicator } from "./workspace-status";

function ProjectRow({
	project,
	expanded,
	onToggle,
	hasTemplate,
	templateWorkspace,
	workspaceCount,
}: {
	project: ListedProject;
	expanded: boolean;
	onToggle: () => void;
	hasTemplate: boolean;
	templateWorkspace: Workspace | undefined;
	workspaceCount: number;
}) {
	const router = useRouter();
	const queryClient = useQueryClient();

	const createWorkspace = useMutation({
		mutationFn: () => createWorkspaceCommand(project.name),
		onSuccess: (workspace) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace created" });
			router.push(
				`/workspace?project=${encodeURIComponent(project.name)}&name=${encodeURIComponent(workspace.name)}`,
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

	const createTemplateMut = useMutation({
		mutationFn: () =>
			invoke<Workspace>("templates_create_template", {
				project: project.name,
			}),
		onSuccess: (workspace) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			toast({ variant: "success", title: "Template workspace created" });
			router.push(
				`/workspace?project=${encodeURIComponent(project.name)}&name=${encodeURIComponent(workspace.name)}`,
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create template",
				description: error.message,
			});
		},
	});

	const editTemplateMut = useMutation({
		mutationFn: () =>
			invoke<Workspace>("templates_edit_template", {
				project: project.name,
			}),
		onSuccess: (workspace) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			toast({ variant: "success", title: "Template workspace created" });
			router.push(
				`/workspace?project=${encodeURIComponent(project.name)}&name=${encodeURIComponent(workspace.name)}`,
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to edit template",
				description: error.message,
			});
		},
	});

	const handleTemplate = () => {
		if (templateWorkspace) {
			router.push(
				`/workspace?project=${encodeURIComponent(templateWorkspace.project ?? "")}&name=${encodeURIComponent(templateWorkspace.name)}`,
			);
		} else if (hasTemplate) {
			editTemplateMut.mutate();
		} else {
			createTemplateMut.mutate();
		}
	};

	const isCreatingTemplate =
		createTemplateMut.isPending || editTemplateMut.isPending;

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
				{workspaceCount > 0 && (
					<ChevronDown
						size={10}
						className={`shrink-0 text-text-placeholder transition-transform ${expanded ? "" : "-rotate-90"}`}
					/>
				)}
			</span>
			<span className="shrink-0 ml-auto flex items-center -mr-1">
				{isCreatingTemplate ? (
					<span className="p-1 flex items-center justify-center">
						<Loader className="text-text-muted" />
					</span>
				) : (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={(e) => {
									e.stopPropagation();
									handleTemplate();
								}}
								className="p-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity"
							>
								<Box size={12} />
							</button>
						</TooltipTrigger>
						<TooltipContent side="right">
							{templateWorkspace ? "Edit Template" : "New Template"}
						</TooltipContent>
					</Tooltip>
				)}
				{hasTemplate &&
					(createWorkspace.isPending ? (
						<span className="p-1 flex items-center justify-center">
							<Loader className="text-text-muted" />
						</span>
					) : (
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={(e) => {
										e.stopPropagation();
										createWorkspace.mutate();
									}}
									className="p-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity"
								>
									<Plus size={12} />
								</button>
							</TooltipTrigger>
							<TooltipContent side="right">New Workspace</TooltipContent>
						</Tooltip>
					))}
			</span>
		</div>
	);
}

function WorkspaceRow({ workspace }: { workspace: Workspace }) {
	const router = useRouter();
	const searchParams = useSearchParams();
	const queryClient = useQueryClient();
	const isActive =
		searchParams.get("name") === workspace.name ||
		searchParams.get("workspace") === workspace.name;
	const isRunning = workspace.status === "RUNNING";
	const isStopped =
		workspace.status === "TERMINATED" || workspace.status === "STOPPED";
	const isTemplate = isTemplateWorkspace(workspace);

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

	const saveTemplateMut = useMutation({
		mutationFn: () =>
			invoke("templates_save_template", { project: workspace.project }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			toast({ variant: "success", title: "Template saved" });
			router.push("/");
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to save template",
				description: error.message,
			});
		},
	});

	const deleteTemplateMut = useMutation({
		mutationFn: () =>
			invoke("templates_delete_template", { project: workspace.project }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			toast({ variant: "success", title: "Template deleted" });
			router.push("/");
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to delete template",
				description: error.message,
			});
		},
	});

	const isStopping =
		workspace.status === "STOPPING" || workspace.status === "SUSPENDING";
	const isStarting =
		workspace.status === "STAGING" || workspace.status === "PROVISIONING";
	const optimisticStopping = stop.isPending || remove.isPending;
	const optimisticStarting = start.isPending;
	const isDisabled =
		optimisticStopping || isStopping || optimisticStarting || isStarting;
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
					? "pointer-events-none"
					: isActive
						? "bg-surface text-text-bright"
						: "text-text-muted hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			<span className="flex items-center gap-2 min-w-0 flex-1">
				<WorkspaceIndicator
					workspace={{
						...workspace,
						isTemplate,
						optimisticStarting,
						optimisticStopping,
					}}
				/>
				<span className={`truncate ${isDisabled ? "opacity-30" : ""}`}>
					{workspaceLabel(workspace)}
				</span>
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
					{isTemplate ? (
						<>
							<button
								type="button"
								onClick={(e) => {
									e.stopPropagation();
									setMenuOpen(false);
									saveTemplateMut.mutate();
								}}
								className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
							>
								<Save size={12} />
								Save
							</button>
							<button
								type="button"
								onClick={(e) => {
									e.stopPropagation();
									setMenuOpen(false);
									router.push("/");
									deleteTemplateMut.mutate();
								}}
								className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-error hover:bg-error/10 rounded transition-colors"
							>
								<Trash2 size={12} />
								Delete
							</button>
						</>
					) : (
						<>
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
						</>
					)}
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
		refetchInterval: 2000,
	});
	const templates = useQuery({
		queryKey: ["templates_list_templates"],
		queryFn: () =>
			invoke<SnapshotTemplate[]>("templates_list_templates", {
				log: "state_changes_only",
				key: "poll:templates_list_templates",
				stateChanged: (previous, next) =>
					JSON.stringify(previous) !== JSON.stringify(next),
			}),
		refetchInterval: 15000,
	});
	const [expanded, setExpanded] = useState<Record<string, boolean>>({});

	if (!projects.data || projects.data.length === 0) return null;

	const isExpanded = (name: string) => expanded[name] !== false;
	const toggle = (name: string) =>
		setExpanded((prev) => ({ ...prev, [name]: !isExpanded(name) }));

	return (
		<aside className="w-48 shrink-0 border-r border-border-light bg-bg flex flex-col">
			<div data-tauri-drag-region className="h-9 shrink-0" />
			<div className="flex-1 overflow-y-auto pb-1">
				{projects.data.map((project) => {
					const projectWorkspaces = (workspaces.data ?? []).filter(
						(w) => w.project === project.name,
					);
					const hasTemplate = (templates.data ?? []).some(
						(t) => t.project === project.name,
					);
					const templateWorkspace = projectWorkspaces.find((w) =>
						isTemplateWorkspace(w),
					);

					return (
						<div key={project.name}>
							<ProjectRow
								project={project}
								expanded={isExpanded(project.name)}
								onToggle={() => toggle(project.name)}
								hasTemplate={hasTemplate}
								templateWorkspace={templateWorkspace}
								workspaceCount={projectWorkspaces.length}
							/>
							{isExpanded(project.name) &&
								projectWorkspaces.map((w) => (
									<WorkspaceRow key={w.name} workspace={w} />
								))}
						</div>
					);
				})}
				<div data-tauri-drag-region className="w-full flex-1" />
			</div>
			<BarFooter />
		</aside>
	);
}
