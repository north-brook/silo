"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
	Box,
	ChevronRight,
	ChevronsUpDown,
	GitBranch,
	PanelLeft,
	Save,
} from "lucide-react";
import Image from "next/image";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { gitUpdateBranch, gitUpdateTargetBranch } from "../lib/git";
import { invoke } from "../lib/invoke";
import type { ListedProject } from "../lib/projects";
import { isTemplateWorkspace, type Workspace } from "../lib/workspaces";
import { GitToggle, useGitBar } from "./git-bar";
import { Loader } from "./loader";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { useProjectsBar } from "./projects-bar";
import { toast } from "./toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

export function TopBar({ workspace }: { workspace: Workspace }) {
	if (isTemplateWorkspace(workspace)) {
		return <TemplateTopBar workspace={workspace} />;
	}

	return <BranchTopBar workspace={workspace} />;
}

function ProjectsBarReopenButton() {
	const { isOpen, toggle } = useProjectsBar();

	if (isOpen) return null;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={toggle}
					className="flex items-center px-1.5 py-0.5 mr-1.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
				>
					<PanelLeft size={12} />
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom">
				<span className="flex items-center gap-1.5">
					Toggle Projects Bar
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

function TemplateTopBar({ workspace }: { workspace: Workspace }) {
	const { isOpen: projectsBarOpen } = useProjectsBar();
	const router = useRouter();
	const queryClient = useQueryClient();

	const projects = useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});
	const projectImage = projects.data?.find(
		(p) => p.name === workspace.project,
	)?.image;

	const save = useMutation({
		mutationFn: () =>
			invoke("templates_save_template", { project: workspace.project ?? "" }),
		onMutate: () => {
			router.push(
				`/workspace/saving?project=${encodeURIComponent(workspace.project ?? "")}&workspace=${encodeURIComponent(workspace.name)}`,
			);
		},
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			queryClient.invalidateQueries({
				queryKey: ["templates_list_templates"],
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

	return (
		<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
			<div
				className={`relative flex items-center justify-between w-full px-3 h-full z-10 ${!projectsBarOpen ? "pl-20" : ""}`}
			>
				<div className="flex items-center gap-2 text-[11px] text-text-muted">
					<ProjectsBarReopenButton />
					{projectImage ? (
						<Image
							width={14}
							height={14}
							src={convertFileSrc(projectImage)}
							alt={workspace.project ?? ""}
							className="rounded object-cover shrink-0"
						/>
					) : (
						<div className="w-3.5 h-3.5 rounded bg-border-light shrink-0" />
					)}
					<ChevronRight size={10} className="shrink-0 text-text-placeholder" />
					<Box size={12} className="shrink-0 text-text-placeholder" />
					<span className="text-text">Template</span>
				</div>
				<div data-tauri-drag-region className="h-full flex-1" />
				{workspace.ready && (
					<button
						type="button"
						disabled={save.isPending}
						onClick={() => save.mutate()}
						className="flex items-center gap-1.5 justify-center px-2.5 py-1 rounded text-[11px] font-medium bg-green-600 text-white transition-colors hover:bg-green-500 disabled:opacity-50 disabled:cursor-not-allowed"
					>
						{save.isPending ? (
							<Loader className="text-white" />
						) : (
							<Save size={10} />
						)}
						Save
					</button>
				)}
			</div>
		</header>
	);
}

function BranchTopBar({ workspace }: { workspace: Workspace }) {
	const { isOpen: projectsBarOpen } = useProjectsBar();
	const { prStatus } = useGitBar();
	const queryClient = useQueryClient();
	const hasPr = prStatus?.status === "open";
	const branchWorkspace = isTemplateWorkspace(workspace) ? null : workspace;

	const [editingBranch, setEditingBranch] = useState(false);
	const [branchDraft, setBranchDraft] = useState(branchWorkspace?.branch ?? "");
	const inputRef = useRef<HTMLInputElement>(null);

	const [targetOpen, setTargetOpen] = useState(false);
	const [optimisticTarget, setOptimisticTarget] = useState(
		branchWorkspace?.target_branch ?? "",
	);

	const projects = useQuery({
		queryKey: ["projects_list_projects"],
		queryFn: () => invoke<ListedProject[]>("projects_list_projects"),
	});
	const projectImage = projects.data?.find(
		(p) => p.name === workspace.project,
	)?.image;

	const branches = useQuery({
		queryKey: ["git_project_branches", workspace.project],
		queryFn: () =>
			invoke<string[]>("git_project_branches", { project: workspace.project }),
		enabled: !!workspace.project && !!branchWorkspace,
	});

	const updateBranch = useMutation({
		mutationFn: (newBranch: string) =>
			gitUpdateBranch(workspace.name, newBranch),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace.name],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
		},
		onError: (error) => {
			setBranchDraft(branchWorkspace?.branch ?? "");
			toast({
				variant: "error",
				title: "Failed to rename branch",
				description: error.message,
			});
		},
	});

	const updateTargetBranch = useMutation({
		mutationFn: (newTargetBranch: string) =>
			gitUpdateTargetBranch(workspace.name, newTargetBranch),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace.name],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
		},
		onError: (error) => {
			setOptimisticTarget(branchWorkspace?.target_branch ?? "");
			toast({
				variant: "error",
				title: "Failed to update target branch",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		if (editingBranch && inputRef.current) {
			inputRef.current.focus();
			inputRef.current.select();
		}
	}, [editingBranch]);

	useEffect(() => {
		setBranchDraft(branchWorkspace?.branch ?? "");
	}, [branchWorkspace?.branch]);

	useEffect(() => {
		setOptimisticTarget(branchWorkspace?.target_branch ?? "");
	}, [branchWorkspace?.target_branch]);

	const PRIORITY_BRANCHES = ["main", "master", "staging", "dev"];

	const sortedBranches = (branches.data ?? []).slice().sort((a, b) => {
		const ai = PRIORITY_BRANCHES.indexOf(a);
		const bi = PRIORITY_BRANCHES.indexOf(b);
		if (ai !== -1 && bi !== -1) return ai - bi;
		if (ai !== -1) return -1;
		if (bi !== -1) return 1;
		return a.localeCompare(b);
	});

	const commitBranch = () => {
		if (!branchWorkspace) {
			setEditingBranch(false);
			return;
		}

		const trimmed = branchDraft.trim();
		setEditingBranch(false);
		if (trimmed && trimmed !== branchWorkspace.branch) {
			updateBranch.mutate(trimmed);
		} else {
			setBranchDraft(branchWorkspace.branch);
		}
	};

	if (!branchWorkspace) return null;

	const targetBranch = optimisticTarget;

	return (
		<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
			<div
				className={`relative flex items-center justify-between w-full pr-2 h-full z-10 ${!projectsBarOpen ? "pl-20" : "pl-3"}`}
			>
				<div className="flex items-center gap-2 text-[11px] text-text-muted">
					<ProjectsBarReopenButton />
					{projectImage ? (
						<Image
							width={14}
							height={14}
							src={convertFileSrc(projectImage)}
							alt={workspace.project ?? ""}
							className="rounded object-cover shrink-0"
						/>
					) : (
						<div className="w-3.5 h-3.5 rounded bg-border-light shrink-0" />
					)}
					<ChevronRight size={10} className="shrink-0 text-text-placeholder" />
					<GitBranch size={12} className="shrink-0 text-text-placeholder" />
					{editingBranch && !hasPr ? (
						<input
							ref={inputRef}
							value={branchDraft}
							onChange={(e) => setBranchDraft(e.target.value)}
							onBlur={commitBranch}
							onKeyDown={(e) => {
								if (e.key === "Enter") commitBranch();
								if (e.key === "Escape") {
									setBranchDraft(branchWorkspace.branch);
									setEditingBranch(false);
								}
							}}
							className="bg-transparent border-0 outline-none text-[11px] text-text-bright p-0 m-0 w-24 rounded-none"
						/>
					) : (
						<button
							type="button"
							disabled={hasPr}
							onClick={() => {
								setBranchDraft(branchWorkspace.branch);
								setEditingBranch(true);
							}}
							className={`transition-colors ${hasPr ? "text-text cursor-default" : "text-text hover:text-text-bright"}`}
						>
							{branchDraft || "branch"}
						</button>
					)}
					<ChevronRight size={10} className="shrink-0 text-text-placeholder" />
					<Popover open={hasPr ? false : targetOpen} onOpenChange={hasPr ? undefined : setTargetOpen}>
						<PopoverTrigger asChild>
							<button
								type="button"
								disabled={hasPr}
								className={`flex items-center gap-1 transition-colors ${hasPr ? "text-text cursor-default" : "text-text hover:text-text-bright"}`}
							>
								{targetBranch || "target branch"}
								{!hasPr && <ChevronsUpDown size={10} className="text-text-placeholder" />}
							</button>
						</PopoverTrigger>
						<PopoverContent
							side="bottom"
							align="start"
							className="w-52 p-1 max-h-64 overflow-y-auto"
						>
							{branches.isLoading && (
								<span className="block px-2 py-1.5 text-xs text-text-muted">
									Loading...
								</span>
							)}
							{sortedBranches.map((b) => (
								<button
									key={b}
									type="button"
									onClick={() => {
										setOptimisticTarget(b);
										updateTargetBranch.mutate(b);
										setTargetOpen(false);
									}}
									className={`block w-full text-left px-2 py-1.5 text-xs rounded transition-colors truncate ${
										b === targetBranch
											? "text-text-bright bg-btn-hover"
											: "text-text hover:bg-btn-hover hover:text-text-bright"
									}`}
								>
									{b}
								</button>
							))}
							{branches.data?.length === 0 && (
								<span className="block px-2 py-1.5 text-xs text-text-muted">
									No branches found
								</span>
							)}
						</PopoverContent>
					</Popover>
				</div>
				<div data-tauri-drag-region className="h-full flex-1" />
				<GitToggle />
			</div>
		</header>
	);
}
