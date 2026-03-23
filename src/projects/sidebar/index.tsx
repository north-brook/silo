import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
	Box,
	ChevronDown,
	Cpu,
	EllipsisVertical,
	FolderOpen,
	PanelLeft,
	Pause,
	Play,
	Plus,
	Save,
	Square,
	Trash2,
} from "lucide-react";
import {
	createContext,
	type ReactNode,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { invoke } from "@/shared/lib/invoke";
import {
	deleteTemplate,
	type ListedProject,
	type SnapshotTemplate,
	type TemplateState,
	saveTemplate,
} from "@/projects/api";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import {
	createWorkspace as createWorkspaceCommand,
	isTemplateWorkspace,
	type Workspace,
	workspaceLabel,
	workspaceIsReady,
} from "@/workspaces/api";
import { LogoIcon } from "@/shared/ui/icons/logo";
import Image from "@/shared/ui/image";
import { Loader } from "@/shared/ui/loader";
import { useNewWorkspace } from "@/projects/sidebar/new-workspace";
import { useOpenProject } from "@/projects/sidebar/open-project";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { toast } from "@/shared/ui/toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { WorkspaceIndicator } from "@/workspaces/layout/status";
import { gitPrStatus, gitPrObserve, gitTreeDirty } from "@/workspaces/git/api";
import {
	type WorkspaceRouteState,
	workspaceHref,
} from "@/workspaces/routes/paths";

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

interface ProjectsSidebarContextValue {
	isOpen: boolean;
	toggle: () => void;
}

const ProjectsSidebarContext = createContext<ProjectsSidebarContextValue>({
	isOpen: true,
	toggle: () => {},
});

export function useProjectsSidebar() {
	return useContext(ProjectsSidebarContext);
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export function ProjectsSidebarProvider({ children }: { children: ReactNode }) {
	const [isOpen, setIsOpen] = useState(true);

	useShortcut<void>({
		event: shortcutEvents.toggleProjectsBar,
		onTrigger: () => {
			setIsOpen((open) => !open);
		},
		onKeyDown: (e) => {
			if (e.metaKey && !e.shiftKey && e.key === "b") {
				e.preventDefault();
				setIsOpen((o) => !o);
			}
		},
	});

	return (
		<ProjectsSidebarContext.Provider
			value={{ isOpen, toggle: () => setIsOpen((o) => !o) }}
		>
			{children}
		</ProjectsSidebarContext.Provider>
	);
}

// ---------------------------------------------------------------------------
// Toggle
// ---------------------------------------------------------------------------

export function ProjectsSidebarToggle() {
	const { toggle } = useProjectsSidebar();

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={toggle}
					className="flex items-center px-1.5 py-0.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
				>
					<PanelLeft size={12} />
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom">
				<span className="flex items-center gap-1.5">
					Toggle Projects Sidebar
					<span className="flex items-center gap-0.5">
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
							⌘
						</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
							B
						</kbd>
					</span>
				</span>
			</TooltipContent>
		</Tooltip>
	);
}

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
	const navigate = useNavigate();
	const queryClient = useQueryClient();

	const createWorkspace = useMutation({
		mutationFn: () => createWorkspaceCommand(project.name),
		onSuccess: (workspace) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace created" });
			navigate(
				workspaceHref({
					project: project.name,
					workspace: workspace.name,
				}),
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
			navigate(
				workspaceHref({
					project: project.name,
					workspace: workspace.name,
				}),
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
			navigate(
				workspaceHref({
					project: project.name,
					workspace: workspace.name,
				}),
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
			navigate(
				workspaceHref({
					project: templateWorkspace.project ?? "",
					workspace: templateWorkspace.name,
				}),
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
							{templateWorkspace
								? "Edit Template"
								: hasTemplate
									? "Edit Template"
									: "New Template"}
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

function WorkspaceRow({
	workspace,
	hotkeyNumber,
}: {
	workspace: Workspace;
	hotkeyNumber?: number;
}) {
	const navigate = useNavigate();
	const { workspace: activeWorkspaceName } = useParams();
	const queryClient = useQueryClient();
	const isActive = activeWorkspaceName === workspace.name;
	const isRunning = workspace.status === "RUNNING";
	const isStopped =
		workspace.status === "TERMINATED" || workspace.status === "STOPPED";
	const isSuspended = workspace.status === "SUSPENDED";
	const isSuspending = workspace.status === "SUSPENDING";
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

	const suspend = useMutation({
		mutationFn: () =>
			invoke("workspaces_suspend_workspace", { workspace: workspace.name }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			toast({ variant: "success", title: "Workspace suspended" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to suspend",
				description: error.message,
			});
		},
	});

	const resume = useMutation({
		mutationFn: () =>
			invoke("workspaces_resume_workspace", { workspace: workspace.name }),
		onMutate: () => {
			navigate(
				workspaceHref({
					project: workspace.project ?? "",
					workspace: workspace.name,
				}),
				{
					state: { transition: "resuming" } satisfies WorkspaceRouteState,
				},
			);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
		},
		onError: (error) => {
			navigate(
				workspaceHref({
					project: workspace.project ?? "",
					workspace: workspace.name,
				}),
				{ replace: true, state: null },
			);
			toast({
				variant: "error",
				title: "Failed to resume",
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
		mutationFn: () => saveTemplate(workspace.project ?? ""),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_get_state", workspace.project ?? ""],
			});
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
		mutationFn: () => deleteTemplate(workspace.project ?? ""),
		onSuccess: (operation) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
			});
			queryClient.setQueryData<TemplateState | undefined>(
				["templates_get_state", workspace.project ?? ""],
				(current) => ({
					project: workspace.project ?? "",
					workspace_name: workspace.name,
					workspace_present: current?.workspace_present ?? true,
					snapshot_name: current?.snapshot_name ?? null,
					operation,
				}),
			);
			toast({ variant: "success", title: "Template deleted" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to delete template",
				description: error.message,
			});
		},
	});

	const isStopping = workspace.status === "STOPPING";
	const optimisticStopping = stop.isPending || remove.isPending;
	const optimisticSuspending = suspend.isPending;
	const optimisticStarting = start.isPending;
	const isDisabled =
		optimisticStopping || optimisticSuspending || isStopping || isSuspending;
	const [menuOpen, setMenuOpen] = useState(false);

	const isReadyBranch =
		!isTemplate && isRunning && workspace.lifecycle.phase === "ready";

	const prStatusQuery = useQuery({
		queryKey: ["git_pr_status", workspace.name],
		queryFn: () => gitPrStatus(workspace.name),
		enabled: isReadyBranch,
		refetchInterval: 10000,
	});

	const hasPr = prStatusQuery.data?.status === "open";

	const observationQuery = useQuery({
		queryKey: ["git_pr_observe", workspace.name],
		queryFn: () => gitPrObserve(workspace.name),
		enabled: isReadyBranch && hasPr,
		refetchInterval: 15000,
	});

	const dirtyQuery = useQuery({
		queryKey: ["git_tree_dirty", workspace.name],
		queryFn: () => gitTreeDirty(workspace.name),
		enabled: isReadyBranch,
		refetchInterval: 5000,
	});

	const openWorkspace = () =>
		navigate(
			workspaceHref({
				project: workspace.project ?? "",
				workspace: workspace.name,
			}),
		);

	return (
		// biome-ignore lint/a11y/useSemanticElements: can't use <button> because it contains interactive children
		<div
			role="button"
			tabIndex={0}
			onClick={() => {
				if (isSuspended && !isTemplate) {
					resume.mutate();
				} else {
					openWorkspace();
				}
			}}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					if (isSuspended && !isTemplate) {
						resume.mutate();
					} else {
						openWorkspace();
					}
				}
			}}
			className={`group flex items-center w-full pl-5 pr-3 py-2 text-xs transition-colors cursor-pointer ${
				isDisabled
					? "pointer-events-none"
					: isActive
						? "bg-btn-hover text-text-bright"
						: "text-text hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			<span className="flex items-center gap-2 min-w-0 flex-1">
				<WorkspaceIndicator
					workspace={{
						...workspace,
						isTemplate,
						optimisticStarting,
						optimisticStopping,
						optimisticSuspending,
						prStatus: prStatusQuery.data ?? null,
						observation: observationQuery.data ?? null,
						dirty: dirtyQuery.data ?? false,
					}}
				/>
				<span className={`truncate ${isDisabled ? "opacity-30" : ""}`}>
					{workspaceLabel(workspace)}
				</span>
			</span>
			{hotkeyNumber !== undefined ? (
				<span className="shrink-0 ml-auto -mr-1 w-5 h-5 flex items-center justify-center text-[10px] font-medium text-text-muted">
					{hotkeyNumber}
				</span>
			) : (
				<Popover open={menuOpen} onOpenChange={setMenuOpen}>
					<PopoverTrigger asChild>
						<button
							type="button"
							onClick={(e) => e.stopPropagation()}
							className={`group/action shrink-0 ml-auto p-1 -mr-1 w-5 h-5 flex items-center justify-center text-text-placeholder hover:text-text-bright transition-opacity ${
								(isRunning &&
									!isTemplate &&
									(workspace.working || workspace.unread)) ||
								(isSuspended && !isTemplate)
									? ""
									: "opacity-0 group-hover:opacity-100"
							}`}
						>
							{isRunning && !isTemplate && workspace.working ? (
								<Loader className="text-blue-400" />
							) : isRunning && !isTemplate && workspace.unread ? (
								<>
									<span className="block w-2 h-2 rounded-full bg-blue-400 group-hover/action:hidden" />
									<EllipsisVertical
										size={12}
										className="hidden group-hover/action:block"
									/>
								</>
							) : isSuspended && !isTemplate ? (
								<>
									<span className="block w-2 h-2 rounded-full bg-yellow-400 group-hover/action:hidden" />
									<EllipsisVertical
										size={12}
										className="hidden group-hover/action:block"
									/>
								</>
							) : (
								<EllipsisVertical size={12} />
							)}
						</button>
					</PopoverTrigger>
					<PopoverContent side="right" align="start" className="w-36 p-1">
						{isTemplate ? (
							<>
								{workspaceIsReady(workspace) && (
									<button
										type="button"
										onClick={(e) => {
											e.stopPropagation();
											setMenuOpen(false);
											saveTemplateMut.mutate();
										}}
										className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
									>
										{saveTemplateMut.isPending ? (
											<Loader className="text-text-bright" />
										) : (
											<Save size={12} />
										)}
										Save
									</button>
								)}
								<button
									type="button"
									onClick={(e) => {
										e.stopPropagation();
										setMenuOpen(false);
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
								{isSuspended && (
									<button
										type="button"
										onClick={(e) => {
											e.stopPropagation();
											setMenuOpen(false);
											resume.mutate();
										}}
										className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
									>
										<Play size={12} />
										Resume
									</button>
								)}
								{isRunning && (
									<button
										type="button"
										onClick={(e) => {
											e.stopPropagation();
											setMenuOpen(false);
											suspend.mutate();
										}}
										className="flex items-center gap-2 w-full px-2 py-1.5 text-xs text-text hover:bg-btn-hover hover:text-text-bright rounded transition-colors"
									>
										<Pause size={12} />
										Suspend
									</button>
								)}
								{isRunning && (
									<button
										type="button"
										onClick={(e) => {
											e.stopPropagation();
											setMenuOpen(false);
											if (isActive) navigate("/");
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
										if (isActive) navigate("/");
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
			)}
		</div>
	);
}

function BarFooter() {
	const openProject = useOpenProject();

	const memory = useQuery({
		queryKey: ["system_memory_usage"],
		queryFn: () =>
			invoke<number>("system_memory_usage", {
				log: "state_changes_only",
				key: "poll:system_memory_usage",
				stateChanged: (previous: number | undefined, next: number) =>
					Math.round((previous ?? 0) / 50) !== Math.round(next / 50),
			}),
		refetchInterval: 5000,
	});

	return (
		<div className="shrink-0">
			<button
				type="button"
				onClick={() => openProject.open()}
				disabled={openProject.isPending}
				className="flex items-center gap-2 w-full px-3 py-2.5 text-xs text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors disabled:opacity-50"
			>
				{openProject.isPending ? (
					<Loader className="text-text-muted" />
				) : (
					<FolderOpen size={12} />
				)}
				Open Project
			</button>
			<div className="px-3 py-2">
				{memory.data !== undefined && (
					<span className="flex items-center gap-1 text-[11px] text-text-muted">
						<Cpu size={10} />
						{memory.data.toFixed(1)} MB
					</span>
				)}
			</div>
		</div>
	);
}

export function ProjectsSidebar() {
	const { isOpen } = useProjectsSidebar();
	const navigate = useNavigate();
	const pathname = useLocation().pathname;
	const isHome = pathname === "/";
	const newWorkspace = useNewWorkspace();
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
	const [metaKeyHeld, setMetaKeyHeld] = useState(false);

	const prevUnreadRef = useRef<Set<string> | null>(null);

	useEffect(() => {
		if (!workspaces.data) return;

		const currentUnread = new Set(
			workspaces.data
				.filter((w) => !isTemplateWorkspace(w) && w.unread)
				.map((w) => w.name),
		);

		const prev = prevUnreadRef.current;

		if (prev !== null) {
			const hasNewUnread = [...currentUnread].some((name) => !prev.has(name));
			if (hasNewUnread) {
				new Audio("/sounds/notification.wav").play().catch(() => {});
			}
		}

		prevUnreadRef.current = currentUnread;
	}, [workspaces.data]);

	const openWorkspaceByHotkey = (digit: number) => {
		if (digit < 0 || digit > 9 || Number.isNaN(digit)) {
			return;
		}

		if (digit === 0) {
			navigate("/");
			return;
		}

		const p = projects.data;
		const w = workspaces.data;
		if (!p || !w) {
			return;
		}

		const flatWorkspaces: Workspace[] = [];
		for (const project of p) {
			const projectWorkspaces = w
				.filter((ws) => ws.project === project.name)
				.sort((a, b) => a.created_at.localeCompare(b.created_at));
			flatWorkspaces.push(...projectWorkspaces);
		}

		const target = flatWorkspaces[digit - 1];
		if (!target) {
			return;
		}

		const isSuspended = target.status === "SUSPENDED";
		const isTemplate = isTemplateWorkspace(target);

		if (isSuspended && !isTemplate) {
			navigate(
				workspaceHref({
					project: target.project ?? "",
					workspace: target.name,
				}),
				{
					state: { transition: "resuming" } satisfies WorkspaceRouteState,
				},
			);
			void invoke("workspaces_resume_workspace", { workspace: target.name })
				.then(() => {
					queryClient.invalidateQueries({
						queryKey: ["workspaces_list_workspaces"],
					});
				})
				.catch((error: Error) => {
					navigate(
						workspaceHref({
							project: target.project ?? "",
							workspace: target.name,
						}),
						{ replace: true, state: null },
					);
					toast({
						variant: "error",
						title: "Failed to resume",
						description: error.message,
					});
				});
			return;
		}

		navigate(
			workspaceHref({
				project: target.project ?? "",
				workspace: target.name,
			}),
		);
	};

	useShortcut<number>({
		event: shortcutEvents.jumpToWorkspace,
		onTrigger: (digit) => {
			openWorkspaceByHotkey(digit);
		},
		onKeyDown: (e, trigger) => {
			if (!e.metaKey || e.shiftKey || e.altKey || e.ctrlKey) return;
			const digit = Number.parseInt(e.key, 10);
			if (digit < 0 || digit > 9 || Number.isNaN(digit)) return;

			e.preventDefault();
			trigger(digit);
		},
	});

	useEffect(() => {
		const down = (e: KeyboardEvent) => {
			if (e.key === "Meta") setMetaKeyHeld(true);
		};
		const up = (e: KeyboardEvent) => {
			if (e.key === "Meta") setMetaKeyHeld(false);
		};
		const blur = () => setMetaKeyHeld(false);
		window.addEventListener("keydown", down);
		window.addEventListener("keyup", up);
		window.addEventListener("blur", blur);
		return () => {
			window.removeEventListener("keydown", down);
			window.removeEventListener("keyup", up);
			window.removeEventListener("blur", blur);
		};
	}, []);

	if (!isOpen) return null;
	if (!projects.data || projects.data.length === 0) return null;

	const isExpanded = (name: string) => expanded[name] !== false;
	const toggle = (name: string) =>
		setExpanded((prev) => ({ ...prev, [name]: !isExpanded(name) }));

	// Build workspace name -> hotkey number (1-9) map matching sidebar order
	const hotkeyMap = new Map<string, number>();
	let hotkeyIndex = 0;
	for (const project of projects.data) {
		const projectWorkspaces = (workspaces.data ?? [])
			.filter((w) => w.project === project.name)
			.sort((a, b) => a.created_at.localeCompare(b.created_at));
		for (const w of projectWorkspaces) {
			hotkeyIndex++;
			if (hotkeyIndex <= 9) hotkeyMap.set(w.name, hotkeyIndex);
		}
	}

	return (
		<aside className="w-48 shrink-0 border-r border-border-light bg-bg flex flex-col">
			<div
				data-tauri-drag-region
				className="h-9 shrink-0 flex items-center justify-end pr-1.5"
			>
				<ProjectsSidebarToggle />
			</div>
			{/* biome-ignore lint/a11y/useSemanticElements: can't use <button> because it contains interactive children */}
			<div
				role="button"
				tabIndex={0}
				onClick={() => navigate("/")}
				onKeyDown={(e) => {
					if (e.key === "Enter" || e.key === " ") {
						e.preventDefault();
						navigate("/");
					}
				}}
				className={`group flex items-center w-full px-3 py-2.5 text-xs transition-colors cursor-pointer ${isHome ? "bg-btn-hover text-text-bright" : "text-text hover:bg-btn-hover hover:text-text-bright"}`}
			>
				<span className="flex items-center gap-2 min-w-0 flex-1">
					<LogoIcon height={12} />
					Dashboard
				</span>
				{metaKeyHeld ? (
					<span className="shrink-0 ml-auto -mr-1 w-5 h-5 flex items-center justify-center text-[10px] font-medium text-text-muted">
						0
					</span>
				) : (
					<span className="shrink-0 ml-auto flex items-center -mr-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<button
									type="button"
									onClick={(e) => {
										e.stopPropagation();
										newWorkspace.open();
									}}
									className="p-1 text-text-placeholder hover:text-text-bright opacity-0 group-hover:opacity-100 transition-opacity"
								>
									<Plus size={12} />
								</button>
							</TooltipTrigger>
							<TooltipContent side="right">New Workspace</TooltipContent>
						</Tooltip>
					</span>
				)}
			</div>
			<div className="flex-1 overflow-y-auto pb-1 mt-0.5 flex flex-col gap-0.5">
				{projects.data.map((project) => {
					const projectWorkspaces = (workspaces.data ?? [])
						.filter((w) => w.project === project.name)
						.sort((a, b) => a.created_at.localeCompare(b.created_at));
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
									<WorkspaceRow
										key={w.name}
										workspace={w}
										hotkeyNumber={
											metaKeyHeld ? hotkeyMap.get(w.name) : undefined
										}
									/>
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
