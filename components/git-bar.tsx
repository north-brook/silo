"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isTauri } from "@tauri-apps/api/core";
import {
	ArrowUpFromLine,
	Ban,
	Check,
	Clock,
	Ellipsis,
	ExternalLink,
	GitMerge,
	GitPullRequestCreateArrow,
	Minus,
	OctagonAlert,
	PanelRight,
	RotateCw,
	SkipForward,
	X,
} from "lucide-react";
import Image from "next/image";
import { useRouter, useSearchParams } from "next/navigation";
import {
	createContext,
	type ReactNode,
	Suspense,
	useContext,
	useEffect,
	useState,
} from "react";
import { cloudSessionHref } from "../lib/cloud";
import {
	type CheckState,
	type Diff,
	type DiffFile,
	type DiffSection,
	gitCreatePr,
	gitDiff,
	gitMergePr,
	gitPrObserve,
	gitPrStatus,
	gitPush,
	gitRerunFailedChecks,
	gitTreeDirty,
	type PullRequestObservation,
	type PullRequestStatus,
} from "../lib/git";
import { invoke } from "../lib/invoke";
import { listenShortcutEvent, shortcutEvents } from "../lib/shortcuts";
import { isTemplateWorkspace, type Workspace } from "../lib/workspaces";
import { Loader } from "./loader";
import { toast } from "./toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

interface GitBarContextValue {
	isOpen: boolean;
	toggle: () => void;
	diff: Diff | null;
	hasChanges: boolean;
	workspace: string;
	project: string;
	isInBranchWorkspace: boolean;
	prStatus: PullRequestStatus | null;
	prStatusLoading: boolean;
	observation: PullRequestObservation | null;
	observationLoading: boolean;
}

const GitBarContext = createContext<GitBarContextValue>({
	isOpen: false,
	toggle: () => {},
	diff: null,
	hasChanges: false,
	workspace: "",
	project: "",
	isInBranchWorkspace: false,
	prStatus: null,
	prStatusLoading: false,
	observation: null,
	observationLoading: false,
});

export function useGitBar() {
	return useContext(GitBarContext);
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

function GitBarProviderInner({ children }: { children: ReactNode }) {
	const searchParams = useSearchParams();
	const workspaceName =
		searchParams.get("name") ?? searchParams.get("workspace") ?? "";
	const project = searchParams.get("project") ?? "";

	const workspaceQuery = useQuery({
		queryKey: ["workspaces_get_workspace", workspaceName],
		queryFn: () =>
			invoke<Workspace>("workspaces_get_workspace", {
				workspace: workspaceName,
			}),
		enabled: !!workspaceName,
	});

	const isInBranchWorkspace =
		!!workspaceName &&
		!!workspaceQuery.data &&
		!isTemplateWorkspace(workspaceQuery.data);
	const isReadyBranchWorkspace =
		isInBranchWorkspace && workspaceQuery.data?.ready === true;

	const diff = useQuery({
		queryKey: ["git_diff", workspaceName],
		queryFn: () => gitDiff(workspaceName),
		enabled: isReadyBranchWorkspace,
		refetchInterval: 5000,
	});

	const hasChanges =
		(diff.data?.overview.additions ?? 0) > 0 ||
		(diff.data?.overview.deletions ?? 0) > 0 ||
		(diff.data?.overview.files_changed ?? 0) > 0;

	const prStatusQuery = useQuery({
		queryKey: ["git_pr_status", workspaceName],
		queryFn: () => gitPrStatus(workspaceName),
		enabled: isReadyBranchWorkspace,
		refetchInterval: 10000,
	});

	const hasPr = prStatusQuery.data?.status === "open";

	const observationQuery = useQuery({
		queryKey: ["git_pr_observe", workspaceName],
		queryFn: () => gitPrObserve(workspaceName),
		enabled: isReadyBranchWorkspace && hasPr,
		refetchInterval: 15000,
	});

	const [isOpen, setIsOpen] = useState(false);

	useEffect(() => {
		if (isTauri()) {
			return listenShortcutEvent<void>(shortcutEvents.toggleGitBar, () => {
				setIsOpen((open) => !open);
			});
		}

		const handler = (e: KeyboardEvent) => {
			if (e.metaKey && e.shiftKey && e.key === "b") {
				e.preventDefault();
				setIsOpen((o) => !o);
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, []);

	const value: GitBarContextValue = {
		isOpen,
		toggle: () => setIsOpen((o) => !o),
		diff: diff.data ?? null,
		hasChanges,
		workspace: workspaceName,
		project,
		isInBranchWorkspace,
		prStatus: prStatusQuery.data ?? null,
		prStatusLoading: prStatusQuery.isLoading,
		observation: observationQuery.data ?? null,
		observationLoading: observationQuery.isLoading,
	};

	return (
		<GitBarContext.Provider value={value}>{children}</GitBarContext.Provider>
	);
}

export function GitBarProvider({ children }: { children: ReactNode }) {
	return (
		<Suspense fallback={children}>
			<GitBarProviderInner>{children}</GitBarProviderInner>
		</Suspense>
	);
}

// ---------------------------------------------------------------------------
// GitToggle
// ---------------------------------------------------------------------------

export function GitToggle() {
	const { isOpen, toggle, diff, hasChanges, isInBranchWorkspace } = useGitBar();

	if (isOpen || !isInBranchWorkspace || !hasChanges) return null;

	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={toggle}
					className="flex items-center gap-2.5 px-1.5 py-0.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
				>
					<span className="flex items-center gap-1.5 text-[11px] font-medium">
						<span className="text-emerald-400">+{additions}</span>
						<span className="text-red-400">-{deletions}</span>
					</span>
					<PanelRight size={12} />
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom">
				<span className="flex items-center gap-1.5">
					Toggle Git Bar
					<span className="flex items-center gap-0.5">
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
							⌘
						</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
							⇧
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

// ---------------------------------------------------------------------------
// GitBar Panel
// ---------------------------------------------------------------------------

export function GitBar() {
	const { isOpen, hasChanges, isInBranchWorkspace } = useGitBar();

	if (!isOpen || !hasChanges || !isInBranchWorkspace) return null;

	return (
		<aside className="w-72 shrink-0 border-l border-border-light bg-bg flex flex-col">
			<GitBarHeader />
			<GitBarTabs />
		</aside>
	);
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

function GitBarHeader() {
	const {
		workspace,
		project,
		toggle,
		prStatus: pr,
		prStatusLoading,
		observation,
	} = useGitBar();
	const router = useRouter();
	const queryClient = useQueryClient();

	const treeDirty = useQuery({
		queryKey: ["git_tree_dirty", workspace],
		queryFn: () => gitTreeDirty(workspace),
		enabled: !!workspace && pr?.status === "open",
		refetchInterval: 5000,
	});

	const createPr = useMutation({
		mutationFn: () => gitCreatePr(workspace),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_status", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_observe", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			router.push(
				cloudSessionHref({
					project,
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
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
			queryClient.invalidateQueries({
				queryKey: ["git_diff", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_tree_dirty", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_observe", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			router.push(
				cloudSessionHref({
					project,
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
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
			router.push("/");
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
		prStatusLoading || (pr?.status === "open" && treeDirty.isLoading);

	const showCreatePr = !isLoading && !pr;
	const showMerge = !isLoading && pr?.status === "open" && !dirty;
	const showPush = !isLoading && pr?.status === "open" && dirty;

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
	const checksRunning = checks.some((c) =>
		pendingCheckStates.includes(c.state),
	);
	const checksFailing = checks.some((c) => failCheckStates.includes(c.state));
	const mergeColor = checksRunning
		? "bg-btn text-text hover:bg-btn-hover"
		: checksFailing
			? "bg-yellow-600 text-white hover:bg-yellow-500"
			: "bg-green-600 text-white hover:bg-green-500";

	useEffect(() => {
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
				merge.mutate();
			}
		};

		if (isTauri()) {
			const unlistenCreateOrPush = listenShortcutEvent<void>(
				shortcutEvents.gitCreateOrPushPr,
				() => {
					runCreatePushOrMerge("create_or_push");
				},
			);
			const unlistenMerge = listenShortcutEvent<void>(
				shortcutEvents.gitMergePr,
				() => {
					runCreatePushOrMerge("merge");
				},
			);
			return () => {
				unlistenCreateOrPush();
				unlistenMerge();
			};
		}

		const handler = (e: KeyboardEvent) => {
			if (!e.metaKey || !e.shiftKey) return;
			if (e.key === "p") {
				e.preventDefault();
				runCreatePushOrMerge("create_or_push");
			}
			if (e.key === "m") {
				e.preventDefault();
				runCreatePushOrMerge("merge");
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [showCreatePr, showPush, showMerge, createPr, push, merge]);

	const hotkeyKbd = (keys: string[]) => (
		<span className="flex items-center gap-0.5">
			{keys.map((k) => (
				<kbd
					key={k}
					className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text"
				>
					{k}
				</kbd>
			))}
		</span>
	);

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
						<span className="flex items-center gap-1.5">
							Toggle Git Bar
							<span className="flex items-center gap-0.5">
								<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
									⌘
								</kbd>
								<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
									⇧
								</kbd>
								<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
									B
								</kbd>
							</span>
						</span>
					</TooltipContent>
				</Tooltip>
				{pr?.status === "open" && (
					<button
						type="button"
						onClick={async () => {
							const { openUrl } = await import("@tauri-apps/plugin-opener");
							openUrl(pr.url);
						}}
						className="flex items-center gap-1 text-[11px] text-text hover:text-text-bright transition-colors"
					>
						PR #{pr.number}
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
							{hotkeyKbd(["⌘", "⇧", "P"])}
						</TooltipContent>
					</Tooltip>
				)}
				{showMerge && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								disabled={merge.isPending}
								onClick={() => merge.mutate()}
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
							{hotkeyKbd(["⌘", "⇧", "M"])}
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
							{hotkeyKbd(["⌘", "⇧", "P"])}
						</TooltipContent>
					</Tooltip>
				)}
			</div>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function GitBarTabs() {
	const { diff, prStatus, observation, observationLoading } = useGitBar();
	const [activeTab, setActiveTab] = useState<"diff" | "checks">("diff");

	const hasPr = prStatus?.status === "open";

	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	const checksIndicator = (() => {
		if (!hasPr) return null;
		if (observationLoading) return <Loader />;
		const checks = observation?.checks ?? [];
		if (checks.length === 0) return null;
		const failStates: CheckState[] = [
			"failure",
			"startup_failure",
			"timed_out",
			"cancelled",
		];
		const pendingStates: CheckState[] = [
			"in_progress",
			"pending",
			"queued",
			"waiting",
			"requested",
		];
		if (checks.some((c) => failStates.includes(c.state)))
			return <X size={10} className="text-red-400" />;
		if (checks.some((c) => pendingStates.includes(c.state))) return <Loader />;
		return <Check size={10} className="text-emerald-400" />;
	})();

	return (
		<>
			<div className="w-full bg-bg shrink-0 flex items-end">
				<button
					type="button"
					onClick={() => setActiveTab("diff")}
					className={`h-9 flex items-center gap-1.5 px-3 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
						activeTab === "diff"
							? "bg-surface text-text-bright border-r-border-light border-b-surface"
							: "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
					}`}
				>
					Diff
					<span className="text-emerald-400">+{additions}</span>
					<span className="text-red-400">-{deletions}</span>
				</button>
				{hasPr && (
					<button
						type="button"
						onClick={() => setActiveTab("checks")}
						className={`h-9 flex items-center gap-1.5 px-3 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
							activeTab === "checks"
								? "bg-surface text-text-bright border-r-border-light border-b-surface"
								: "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
						}`}
					>
						Checks
						{checksIndicator}
					</button>
				)}
				<div className="flex-1 h-9 border-b border-border-light" />
			</div>
			<div className="flex-1 overflow-y-auto bg-surface">
				{activeTab === "diff" && <DiffTab />}
				{activeTab === "checks" && hasPr && (
					<ChecksTab observation={observation} isLoading={observationLoading} />
				)}
			</div>
		</>
	);
}

// ---------------------------------------------------------------------------
// Diff Tab
// ---------------------------------------------------------------------------

function DiffTab() {
	const { diff } = useGitBar();

	if (!diff) return null;

	return (
		<div className="flex flex-col gap-2">
			<DiffSectionView label="Local" section={diff.local} />
			<DiffSectionView label="Remote" section={diff.remote} />
		</div>
	);
}

function DiffSectionView({
	label,
	section,
}: {
	label: string;
	section: DiffSection;
}) {
	if (section.files.length === 0) return null;

	return (
		<div>
			<div className="flex items-center justify-between px-3 py-1.5 text-[11px]">
				<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
					{label}
				</span>
				<span className="text-text-muted">
					{section.overview.additions > 0 && (
						<span className="text-emerald-400">
							+{section.overview.additions}
						</span>
					)}
					{section.overview.additions > 0 &&
						section.overview.deletions > 0 &&
						" "}
					{section.overview.deletions > 0 && (
						<span className="text-red-400">-{section.overview.deletions}</span>
					)}
				</span>
			</div>
			{section.files.map((file) => (
				<DiffFileRow key={file.path} file={file} />
			))}
		</div>
	);
}

function DiffFileRow({ file }: { file: DiffFile }) {
	return (
		<div className="flex items-center justify-between px-3 py-1 text-[11px] hover:bg-btn-hover transition-colors">
			<span className="truncate min-w-0 text-text">{file.path}</span>
			<span className="shrink-0 ml-2 text-text-muted">
				{file.additions > 0 && (
					<span className="text-emerald-400">+{file.additions}</span>
				)}
				{file.additions > 0 && file.deletions > 0 && " "}
				{file.deletions > 0 && (
					<span className="text-red-400">-{file.deletions}</span>
				)}
			</span>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Checks Tab
// ---------------------------------------------------------------------------

function ChecksTab({
	observation,
	isLoading,
}: {
	observation: PullRequestObservation | null;
	isLoading: boolean;
}) {
	const { workspace } = useGitBar();
	const queryClient = useQueryClient();

	const rerunFailed = useMutation({
		mutationFn: () => gitRerunFailedChecks(workspace),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_observe", workspace],
			});
		},
	});

	if (isLoading || !observation) {
		return (
			<div className="flex items-center justify-center py-8">
				<Loader />
			</div>
		);
	}

	const data = observation;

	const pendingStates: CheckState[] = [
		"in_progress",
		"pending",
		"queued",
		"waiting",
		"requested",
	];
	const failStates: CheckState[] = ["failure", "startup_failure", "timed_out"];
	const allResolved =
		data.checks.length > 0 &&
		!data.checks.some((c) => pendingStates.includes(c.state));
	const hasFailed = data.checks.some((c) => failStates.includes(c.state));

	return (
		<div className="flex flex-col gap-3 p-3">
			{/* PR info */}
			{data.title && (
				<div>
					<p className="text-[11px] font-medium text-text-bright">
						{data.title}
					</p>
					{data.body && (
						<p className="text-[11px] text-text-muted mt-1 line-clamp-3">
							{data.body}
						</p>
					)}
				</div>
			)}

			{/* Deployments */}
			{data.deployments.length > 0 && (
				<div className="flex flex-col gap-1">
					<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
						Deployments
					</span>
					{data.deployments.map((d) => (
						<button
							key={d.id}
							type="button"
							onClick={async () => {
								if (!d.url) return;
								const { openUrl } = await import("@tauri-apps/plugin-opener");
								openUrl(d.url);
							}}
							className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
						>
							{d.icon_url ? (
								<Image
									src={d.icon_url}
									alt=""
									unoptimized
									width={14}
									height={14}
									className="w-3.5 h-3.5 rounded shrink-0"
								/>
							) : (
								<StatusDot state={d.state} />
							)}
							<span className="text-text truncate flex-1 min-w-0">
								{d.url ? new URL(d.url).hostname : d.environment}
							</span>
						</button>
					))}
				</div>
			)}

			{/* Checks */}
			{data.checks.length > 0 && (
				<div className="flex flex-col gap-1">
					<div className="flex items-center justify-between">
						<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
							Checks
						</span>
						{allResolved && hasFailed && (
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										disabled={rerunFailed.isPending}
										onClick={() => rerunFailed.mutate()}
										className="flex items-center text-text-muted hover:text-text transition-colors disabled:opacity-50"
									>
										{rerunFailed.isPending ? <Loader /> : <RotateCw size={9} />}
									</button>
								</TooltipTrigger>
								<TooltipContent side="left">Rerun failed</TooltipContent>
							</Tooltip>
						)}
					</div>
					{[...data.checks]
						.sort((a, b) => a.name.localeCompare(b.name))
						.map((c) => (
							<button
								key={c.id}
								type="button"
								onClick={async () => {
									if (!c.link) return;
									const { openUrl } = await import("@tauri-apps/plugin-opener");
									openUrl(c.link);
								}}
								className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
							>
								<CheckStateIcon state={c.state} />
								<span className="text-text truncate">{c.name}</span>
							</button>
						))}
				</div>
			)}

			{data.deployments.length === 0 && data.checks.length === 0 && (
				<p className="text-[11px] text-text-muted py-4 text-center">
					No checks or deployments
				</p>
			)}
		</div>
	);
}

function StatusDot({ state }: { state: string }) {
	let color: string;
	switch (state) {
		case "success":
			color = "bg-emerald-400";
			break;
		case "failure":
		case "error":
			color = "bg-red-400";
			break;
		case "pending":
		case "in_progress":
		case "queued":
			color = "bg-yellow-400";
			break;
		default:
			color = "bg-text-muted";
			break;
	}
	return <span className={`shrink-0 w-1.5 h-1.5 rounded-full ${color}`} />;
}

function CheckStateIcon({ state }: { state: CheckState }) {
	const size = 12;
	switch (state) {
		case "success":
			return <Check size={size} className="shrink-0 text-emerald-400" />;
		case "failure":
		case "startup_failure":
			return <X size={size} className="shrink-0 text-red-400" />;
		case "timed_out":
			return <Clock size={size} className="shrink-0 text-red-400" />;
		case "cancelled":
			return <Ban size={size} className="shrink-0 text-text-muted" />;
		case "skipped":
			return <SkipForward size={size} className="shrink-0 text-text-muted" />;
		case "action_required":
			return <OctagonAlert size={size} className="shrink-0 text-yellow-400" />;
		case "in_progress":
			return <Loader className="shrink-0" />;
		case "queued":
		case "waiting":
		case "requested":
		case "pending":
			return <Ellipsis size={size} className="shrink-0 text-yellow-400" />;
		case "neutral":
			return <Minus size={size} className="shrink-0 text-text-muted" />;
		default:
			return <Minus size={size} className="shrink-0 text-text-muted" />;
	}
}
