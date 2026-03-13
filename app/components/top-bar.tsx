"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, ChevronsUpDown, GitBranch, Save } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { invoke } from "../../lib/invoke";
import { isTemplateWorkspace, type Workspace } from "../../lib/workspaces";
import { Loader } from "./loader";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { toast } from "./toaster";

export function TopBar({ workspace }: { workspace: Workspace }) {
	if (isTemplateWorkspace(workspace)) {
		return <TemplateTopBar workspace={workspace} />;
	}

	return <BranchTopBar workspace={workspace} />;
}

function TemplateTopBar({ workspace }: { workspace: Workspace }) {
	const router = useRouter();
	const queryClient = useQueryClient();

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
			<div className="relative flex items-center justify-between w-full px-3 h-full z-10">
				<span className="text-[11px] text-text-muted">
					<span className="text-text">{workspace.project}</span> template
				</span>
				<div data-tauri-drag-region className="h-full flex-1" />
				<button
					type="button"
					disabled={save.isPending}
					onClick={() => save.mutate()}
					className="flex items-center gap-1.5 justify-center px-3 py-0.5 rounded text-[11px] font-medium bg-green-600 text-white transition-colors hover:bg-green-500 disabled:opacity-50 disabled:cursor-not-allowed"
				>
					{save.isPending ? <Loader className="text-white" /> : <Save size={10} />}
					Save
				</button>
			</div>
		</header>
	);
}

function BranchTopBar({ workspace }: { workspace: Workspace }) {
	const queryClient = useQueryClient();
	const branchWorkspace = isTemplateWorkspace(workspace) ? null : workspace;

	const [editingBranch, setEditingBranch] = useState(false);
	const [branchDraft, setBranchDraft] = useState(branchWorkspace?.branch ?? "");
	const inputRef = useRef<HTMLInputElement>(null);

	const [targetOpen, setTargetOpen] = useState(false);

	const branches = useQuery({
		queryKey: ["git_project_branches", workspace.project],
		queryFn: () =>
			invoke<string[]>("git_project_branches", { project: workspace.project }),
		enabled: !!workspace.project && !!branchWorkspace,
	});

	const updateBranch = useMutation({
		mutationFn: (newBranch: string) =>
			invoke("workspaces_update_workspace_branch", {
				workspace: workspace.name,
				branch: newBranch,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace.name],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
		},
	});

	const updateTargetBranch = useMutation({
		mutationFn: (newTargetBranch: string) =>
			invoke("workspaces_update_workspace_target_branch", {
				workspace: workspace.name,
				target_branch: newTargetBranch,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace.name],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
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

	const targetBranch = branchWorkspace.target_branch;

	return (
		<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
			<div data-tauri-drag-region className="absolute inset-0" />
			<div className="relative flex items-center gap-1.5 px-3 text-[11px] text-text-muted z-10">
				<GitBranch size={12} className="shrink-0 text-text-placeholder" />
				{editingBranch ? (
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
						onClick={() => {
							setBranchDraft(branchWorkspace.branch);
							setEditingBranch(true);
						}}
						className="text-text hover:text-text-bright transition-colors"
					>
						{branchWorkspace.branch || "branch"}
					</button>
				)}
				<ChevronRight size={10} className="shrink-0 text-text-placeholder" />
				<Popover open={targetOpen} onOpenChange={setTargetOpen}>
					<PopoverTrigger asChild>
						<button
							type="button"
							className="flex items-center gap-1 text-text hover:text-text-bright transition-colors"
						>
							{targetBranch || "target branch"}
							<ChevronsUpDown size={10} className="text-text-placeholder" />
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
		</header>
	);
}
