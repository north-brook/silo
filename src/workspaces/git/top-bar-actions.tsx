import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowUpFromLine,
	GitMerge,
	GitPullRequestClosed,
	GitPullRequestCreateArrow,
	X,
} from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@/shared/lib/invoke";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "@/shared/ui/dialog";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
	gitCreatePr,
	gitMergePr,
	gitPrDetails,
	gitPush,
	gitResolveConflicts,
	gitTreeDirty,
} from "@/workspaces/git/api";
import {
	CheckStateIcon,
	GitChecksStatusIndicator,
} from "@/workspaces/git/checks";
import { useGitSidebar } from "@/workspaces/git/context";
import {
	type SessionRouteState,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";
import { useWorkspaceReady } from "@/workspaces/state";

export function GitTopBarActions() {
	const {
		isOpen,
		isInBranchWorkspace,
		workspace,
		project,
		diff,
		hasChanges,
		openTab,
		prSummary,
		prSummaryLoading,
	} = useGitSidebar();
	const isReady = useWorkspaceReady();
	const navigate = useNavigate();
	const queryClient = useQueryClient();

	const hasPr = prSummary?.status === "open";

	const treeDirty = useQuery({
		queryKey: ["git_tree_dirty", workspace],
		queryFn: () => gitTreeDirty(workspace),
		enabled: !!workspace && hasPr && !isOpen,
		refetchInterval: 5000,
	});

	const createPr = useMutation({
		mutationFn: () => gitCreatePr(workspace),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_summary", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_details", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_deployments", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				workspaceSessionHref({
					project,
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create PR",
				description: error.message,
			});
		},
	});

	const push = useMutation({
		mutationFn: () => gitPush(workspace),
		onSuccess: (result) => {
			queryClient.invalidateQueries({ queryKey: ["git_diff", workspace] });
			queryClient.invalidateQueries({
				queryKey: ["git_tree_dirty", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_summary", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_details", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_deployments", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				workspaceSessionHref({
					project,
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to push",
				description: error.message,
			});
		},
	});

	const merge = useMutation({
		mutationFn: () => gitMergePr(workspace),
		onSuccess: async () => {
			await invoke("workspaces_delete_workspace", { workspace });
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces"],
			});
			navigate("/");
			toast({ variant: "success", title: "PR merged" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to merge PR",
				description: error.message,
			});
		},
	});

	const resolveConflicts = useMutation({
		mutationFn: () => gitResolveConflicts(workspace),
		onSuccess: (result) => {
			queryClient.invalidateQueries({ queryKey: ["git_diff", workspace] });
			queryClient.invalidateQueries({
				queryKey: ["git_tree_dirty", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_summary", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_details", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_deployments", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				workspaceSessionHref({
					project,
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to resolve conflicts",
				description: error.message,
			});
		},
	});

	const [mergeConfirmOpen, setMergeConfirmOpen] = useState(false);
	const dirty = treeDirty.data ?? true;
	const isLoading = prSummaryLoading || (hasPr && treeDirty.isLoading);
	const mergeability = prSummary?.mergeability ?? null;
	const mergeabilityUnknown =
		hasPr && (mergeability == null || mergeability === "unknown");
	const hasMergeConflicts = mergeability === "conflicting";

	const showCreatePr = !isLoading && !prSummary && hasChanges;
	const showResolveConflicts = !isLoading && hasPr && hasMergeConflicts;
	const showMerge =
		!isLoading && hasPr && mergeability === "mergeable" && !dirty;
	const showCheckingMerge =
		!isLoading && hasPr && mergeabilityUnknown && !dirty && !hasMergeConflicts;
	const showPush = !isLoading && hasPr && dirty && !hasMergeConflicts;
	const mergeDetailsQuery = useQuery({
		queryKey: ["git_pr_details", workspace, prSummary?.head_ref_oid ?? null],
		queryFn: () => gitPrDetails(workspace),
		enabled: !!workspace && mergeConfirmOpen && showMerge,
		refetchInterval: 15000,
	});

	const checks = mergeDetailsQuery.data?.checks ?? [];
	const checksKnown = prSummary?.checks != null;
	const checksRunning = prSummary?.checks?.has_pending ?? false;
	const checksFailing = prSummary?.checks?.has_failing ?? false;
	const mergeColor =
		!checksKnown || checksRunning
			? "bg-btn text-text hover:bg-btn-hover"
			: checksFailing
				? "bg-yellow-600 text-white hover:bg-yellow-500"
				: "bg-green-600 text-white hover:bg-green-500";

	const handleMerge = () => {
		if (!checksKnown || checksRunning || checksFailing) {
			setMergeConfirmOpen(true);
			return;
		}
		merge.mutate();
	};

	const runCreatePushOrMerge = (action: "create_or_push" | "merge") => {
		if (action === "create_or_push") {
			if (showCreatePr && !createPr.isPending) {
				createPr.mutate();
			} else if (showPush && !push.isPending) {
				push.mutate();
			}
			return;
		}
		if (showResolveConflicts && !resolveConflicts.isPending) {
			resolveConflicts.mutate();
			return;
		}
		if (showMerge && !merge.isPending) {
			handleMerge();
		}
	};

	useShortcut<void>({
		event: shortcutEvents.gitCreateOrPushPr,
		onTrigger: () => {
			runCreatePushOrMerge("create_or_push");
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey) return;
			if (e.key === "p") {
				e.preventDefault();
				runCreatePushOrMerge("create_or_push");
			}
		},
	});

	useShortcut<void>({
		event: shortcutEvents.gitMergePr,
		onTrigger: () => {
			runCreatePushOrMerge("merge");
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey) return;
			if (e.key === "m") {
				e.preventDefault();
				runCreatePushOrMerge("merge");
			}
		},
	});

	if (isOpen || !isInBranchWorkspace || !isReady) return null;

	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	return (
		<>
			<div className="flex items-center gap-1 mr-1">
				{(hasChanges || hasPr) && (
					<div className="flex items-center">
						{hasChanges && (
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => openTab("diff")}
										className="flex items-center justify-center gap-1.5 h-5 px-1.5 rounded text-sm font-medium text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
									>
										<span className="text-emerald-400">+{additions}</span>
										<span className="text-red-400">-{deletions}</span>
									</button>
								</TooltipTrigger>
								<TooltipContent side="bottom">
									<span className="flex items-center gap-1.5">
										Diff
										<HotkeyHint keys={["⌘", "⇧", "D"]} />
									</span>
								</TooltipContent>
							</Tooltip>
						)}
						{hasPr && (
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										onClick={() => openTab("checks")}
										className="flex items-center justify-center h-5 px-1.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
									>
										<GitChecksStatusIndicator
											checks={prSummary?.checks ?? null}
											isLoading={prSummaryLoading}
										/>
									</button>
								</TooltipTrigger>
								<TooltipContent side="bottom">
									<span className="flex items-center gap-1.5">
										Checks
										<HotkeyHint keys={["⌘", "⇧", "C"]} />
									</span>
								</TooltipContent>
							</Tooltip>
						)}
					</div>
				)}
				{isLoading && <Loader />}
				{showCreatePr && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={createPr.isPending}
								onClick={() => createPr.mutate()}
								className="flex items-center gap-1.5 px-2.5 py-1 rounded text-sm font-medium bg-green-600 text-white hover:bg-green-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
							>
								{createPr.isPending ? (
									<Loader className="text-white" />
								) : (
									<GitPullRequestCreateArrow size={10} />
								)}
								Open PR
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							<HotkeyHint keys={["⌘", "⇧", "P"]} />
						</TooltipContent>
					</Tooltip>
				)}
				{showMerge && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={merge.isPending}
								onClick={handleMerge}
								className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-sm font-medium ${mergeColor} transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
							>
								{merge.isPending ? (
									<Loader className={checksRunning ? "" : "text-white"} />
								) : (
									<GitMerge size={10} />
								)}
								Merge
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							<HotkeyHint keys={["⌘", "⇧", "M"]} />
						</TooltipContent>
					</Tooltip>
				)}
				{showResolveConflicts && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={resolveConflicts.isPending}
								onClick={() => resolveConflicts.mutate()}
								className="flex items-center gap-1.5 px-2.5 py-1 rounded text-sm font-medium bg-red-600 text-white hover:bg-red-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
							>
								{resolveConflicts.isPending ? (
									<Loader className="text-white" />
								) : (
									<GitPullRequestClosed size={10} />
								)}
								Resolve conflicts
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							<HotkeyHint keys={["⌘", "⇧", "M"]} />
						</TooltipContent>
					</Tooltip>
				)}
				{showCheckingMerge && (
					<button
						type="button"
						disabled
						className="flex items-center gap-1.5 px-2.5 py-1 rounded text-sm font-medium bg-btn text-text transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
					>
						<Loader />
						Checking merge
					</button>
				)}
				{showPush && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={push.isPending}
								onClick={() => push.mutate()}
								className="flex items-center gap-1.5 px-2.5 py-1 rounded text-sm font-medium bg-btn text-text hover:bg-btn-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
							>
								{push.isPending ? <Loader /> : <ArrowUpFromLine size={10} />}
								Push
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							<HotkeyHint keys={["⌘", "⇧", "P"]} />
						</TooltipContent>
					</Tooltip>
				)}
			</div>

			<Dialog open={mergeConfirmOpen} onOpenChange={setMergeConfirmOpen}>
				<DialogContent
					className="max-w-sm"
					onKeyDown={(e) => {
						if (e.metaKey && e.key === "Enter") {
							e.preventDefault();
							merge.mutate();
							setMergeConfirmOpen(false);
						}
					}}
				>
					<DialogHeader className="flex flex-row items-center justify-between">
						<DialogTitle>
							{!checksKnown
								? "Merge without loaded checks?"
								: checksFailing
									? "Merge with failing checks?"
									: "Merge with running checks?"}
						</DialogTitle>
						<DialogClose className="text-text-muted hover:text-text transition-colors">
							<X size={14} />
						</DialogClose>
					</DialogHeader>

					<div className="mt-3 flex flex-col gap-0.5">
						{mergeDetailsQuery.isLoading ? (
							<div className="flex items-center justify-center py-4">
								<Loader />
							</div>
						) : mergeDetailsQuery.isError ? (
							<p className="py-2 text-sm text-text-muted">
								Failed to load checks.{" "}
								{queryErrorMessage(mergeDetailsQuery.error)}
							</p>
						) : (
							[...checks]
								.sort((left, right) => left.name.localeCompare(right.name))
								.map((check) => (
									<div
										key={check.id}
										className="flex items-center gap-2 px-1 py-1 text-sm"
									>
										<CheckStateIcon state={check.state} />
										<span className="text-text truncate">{check.name}</span>
									</div>
								))
						)}
					</div>

					<button
						type="button"
						onClick={() => {
							merge.mutate();
							setMergeConfirmOpen(false);
						}}
						className="mt-4 w-full flex items-center justify-center gap-1.5 px-2.5 py-1.5 rounded text-sm font-medium bg-yellow-600 text-white hover:bg-yellow-500 transition-colors"
					>
						<GitMerge size={10} />
						Merge
					</button>
				</DialogContent>
			</Dialog>
		</>
	);
}

function HotkeyHint({ keys }: { keys: string[] }) {
	return (
		<span className="flex items-center gap-0.5">
			{keys.map((key) => (
				<kbd
					key={key}
					className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text"
				>
					{key}
				</kbd>
			))}
		</span>
	);
}

function queryErrorMessage(error: unknown): string {
	return error instanceof Error && error.message ? error.message : "";
}
