import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowUpFromLine,
	ExternalLink,
	GitMerge,
	GitPullRequestCreateArrow,
	PanelRight,
	X,
} from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import type { CheckState } from "@/workspaces/git/api";
import {
	gitCreatePr,
	gitMergePr,
	gitPush,
	gitTreeDirty,
} from "@/workspaces/git/api";
import { CheckStateIcon } from "@/workspaces/git/checks";
import { useGitSidebar } from "@/workspaces/git/context";
import { invoke } from "@/shared/lib/invoke";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import {
	type SessionRouteState,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";
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

export function GitSidebarHeader() {
	const {
		workspace,
		project,
		toggle,
		prStatus,
		prStatusLoading,
		observation,
	} = useGitSidebar();
	const navigate = useNavigate();
	const queryClient = useQueryClient();

	const treeDirty = useQuery({
		queryKey: ["git_tree_dirty", workspace],
		queryFn: () => gitTreeDirty(workspace),
		enabled: !!workspace && prStatus?.status === "open",
		refetchInterval: 5000,
	});

	const createPr = useMutation({
		mutationFn: () => gitCreatePr(workspace),
		onSuccess: (result) => {
			queryClient.invalidateQueries({ queryKey: ["git_pr_status", workspace] });
			queryClient.invalidateQueries({ queryKey: ["git_pr_observe", workspace] });
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
			queryClient.invalidateQueries({ queryKey: ["git_tree_dirty", workspace] });
			queryClient.invalidateQueries({ queryKey: ["git_pr_observe", workspace] });
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

	const dirty = treeDirty.data ?? true;
	const isLoading =
		prStatusLoading || (prStatus?.status === "open" && treeDirty.isLoading);

	const showCreatePr = !isLoading && !prStatus;
	const showMerge = !isLoading && prStatus?.status === "open" && !dirty;
	const showPush = !isLoading && prStatus?.status === "open" && dirty;

	const checks = observation?.checks ?? [];
	const pendingCheckStates: CheckState[] = [
		"in_progress",
		"pending",
		"queued",
		"waiting",
		"requested",
	];
	const failCheckStates: CheckState[] = [
		"failure",
		"startup_failure",
		"timed_out",
	];
	const checksRunning = checks.some((check) =>
		pendingCheckStates.includes(check.state),
	);
	const checksFailing = checks.some((check) =>
		failCheckStates.includes(check.state),
	);
	const mergeColor = checksRunning
		? "bg-btn text-text hover:bg-btn-hover"
		: checksFailing
			? "bg-yellow-600 text-white hover:bg-yellow-500"
			: "bg-green-600 text-white hover:bg-green-500";

	const [mergeConfirmOpen, setMergeConfirmOpen] = useState(false);

	const handleMerge = () => {
		if (checksRunning || checksFailing) {
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

	return (
		<div className="h-9 flex items-center justify-between pl-1.5 pr-3 border-b border-border-light shrink-0">
			<div className="flex items-center gap-2">
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={toggle}
							className="flex items-center px-1.5 py-0.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
						>
							<PanelRight size={12} />
						</button>
					</TooltipTrigger>
					<TooltipContent side="bottom">
						<HotkeyHint keys={["⌘", "⌥", "B"]}>
							Toggle Git Sidebar
						</HotkeyHint>
					</TooltipContent>
				</Tooltip>
				{prStatus?.status === "open" && (
					<button
						type="button"
						onClick={async () => {
							const { openUrl } = await import("@tauri-apps/plugin-opener");
							openUrl(prStatus.url);
						}}
						className="flex items-center gap-1 text-[11px] text-text hover:text-text-bright transition-colors"
					>
						PR #{prStatus.number}
						<ExternalLink size={10} />
					</button>
				)}
			</div>
			<div>
				{isLoading && <Loader />}
				{showCreatePr && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={createPr.isPending}
								onClick={() => createPr.mutate()}
								className="flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium bg-green-600 text-white hover:bg-green-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
							>
								{createPr.isPending ? (
									<Loader className="text-white" />
								) : (
									<GitPullRequestCreateArrow size={10} />
								)}
								Open PR
							</button>
						</TooltipTrigger>
						<TooltipContent side="left">
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
								className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium ${mergeColor} transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
							>
								{merge.isPending ? (
									<Loader className={checksRunning ? "" : "text-white"} />
								) : (
									<GitMerge size={10} />
								)}
								Merge
							</button>
						</TooltipTrigger>
						<TooltipContent side="left">
							<HotkeyHint keys={["⌘", "⇧", "M"]} />
						</TooltipContent>
					</Tooltip>
				)}
				{showPush && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={push.isPending}
								onClick={() => push.mutate()}
								className="flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium bg-btn text-text hover:bg-btn-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
							>
								{push.isPending ? <Loader /> : <ArrowUpFromLine size={10} />}
								Push
							</button>
						</TooltipTrigger>
						<TooltipContent side="left">
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
							{checksFailing
								? "Merge with failing checks?"
								: "Merge with running checks?"}
						</DialogTitle>
						<DialogClose className="text-text-muted hover:text-text transition-colors">
							<X size={14} />
						</DialogClose>
					</DialogHeader>

					<div className="mt-3 flex flex-col gap-0.5">
						{[...checks]
							.sort((left, right) => left.name.localeCompare(right.name))
							.map((check) => (
								<div
									key={check.id}
									className="flex items-center gap-2 px-1 py-1 text-[11px]"
								>
									<CheckStateIcon state={check.state} />
									<span className="text-text truncate">{check.name}</span>
								</div>
							))}
					</div>

					<button
						type="button"
						onClick={() => {
							merge.mutate();
							setMergeConfirmOpen(false);
						}}
						className="mt-4 w-full flex items-center justify-center gap-1.5 px-2.5 py-1.5 rounded text-[11px] font-medium bg-yellow-600 text-white hover:bg-yellow-500 transition-colors"
					>
						<GitMerge size={10} />
						Merge
					</button>
				</DialogContent>
			</Dialog>
		</div>
	);
}

function HotkeyHint({
	keys,
	children,
}: {
	keys: string[];
	children?: React.ReactNode;
}) {
	return (
		<span className="flex items-center gap-1.5">
			{children}
			<span className="flex items-center gap-0.5">
				{keys.map((key) => (
					<kbd
						key={key}
						className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text"
					>
						{key}
					</kbd>
				))}
			</span>
		</span>
	);
}
