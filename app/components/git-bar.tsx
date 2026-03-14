"use client";

import {
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
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
	SkipForward,
	X,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import {
	createContext,
	Suspense,
	useContext,
	useEffect,
	useState,
	type ReactNode,
} from "react";
import {
	gitCreatePr,
	gitDiff,
	gitMergePr,
	gitPrObserve,
	gitPrStatus,
	gitPush,
	gitTreeDirty,
	type CheckState,
	type Diff,
	type DiffFile,
	type DiffSection,
} from "../../lib/git";
import { invoke } from "../../lib/invoke";
import { isTemplateWorkspace, type Workspace } from "../../lib/workspaces";
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
}

const GitBarContext = createContext<GitBarContextValue>({
	isOpen: false,
	toggle: () => {},
	diff: null,
	hasChanges: false,
	workspace: "",
	project: "",
	isInBranchWorkspace: false,
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

	const diff = useQuery({
		queryKey: ["git_diff", workspaceName],
		queryFn: () => gitDiff(workspaceName),
		enabled: isInBranchWorkspace,
		refetchInterval: 5000,
	});

	const hasChanges =
		(diff.data?.overview.additions ?? 0) > 0 ||
		(diff.data?.overview.deletions ?? 0) > 0 ||
		(diff.data?.overview.files_changed ?? 0) > 0;

	const [isOpen, setIsOpen] = useState(false);

	useEffect(() => {
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
	const { isOpen, toggle, diff, hasChanges, isInBranchWorkspace } =
		useGitBar();

	if (!isInBranchWorkspace || !hasChanges) return null;

	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={toggle}
					className={`flex items-center px-1.5 py-0.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors ${isOpen ? "" : "gap-1.5"}`}
				>
					<span className={`flex items-center gap-1.5 text-[11px] font-medium transition-opacity overflow-hidden ${isOpen ? "opacity-0 w-0" : "opacity-100"}`}>
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
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text-muted">⌘</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text-muted">⇧</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text-muted">B</kbd>
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
	const { workspace, project } = useGitBar();
	const router = useRouter();
	const queryClient = useQueryClient();

	const prStatus = useQuery({
		queryKey: ["git_pr_status", workspace],
		queryFn: () => gitPrStatus(workspace),
		enabled: !!workspace,
		refetchInterval: 10000,
	});

	const treeDirty = useQuery({
		queryKey: ["git_tree_dirty", workspace],
		queryFn: () => gitTreeDirty(workspace),
		enabled: !!workspace && prStatus.data?.status === "open",
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
			router.push(
				`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspace)}&terminal=${encodeURIComponent(result.terminal)}`,
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
			router.push(
				`/workspace/terminal?project=${encodeURIComponent(project)}&workspace=${encodeURIComponent(workspace)}&terminal=${encodeURIComponent(result.terminal)}`,
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
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_status", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_diff", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_observe", workspace],
			});
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

	const pr = prStatus.data;
	const dirty = treeDirty.data ?? true;
	const isLoading =
		prStatus.isLoading ||
		(pr?.status === "open" && treeDirty.isLoading);

	return (
		<div className="h-9 flex items-center justify-between px-3 border-b border-border-light shrink-0">
			<div>
				{pr?.status === "open" && (
					<button
						type="button"
						onClick={async () => {
							const { openUrl } = await import(
								"@tauri-apps/plugin-opener"
							);
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
				{!isLoading && !pr && (
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
				)}
				{!isLoading && pr?.status === "open" && !dirty && (
					<button
						type="button"
						disabled={merge.isPending}
						onClick={() => merge.mutate()}
						className="flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium bg-green-600 text-white hover:bg-green-500 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
					>
						{merge.isPending ? (
							<Loader className="text-white" />
						) : (
							<GitMerge size={10} />
						)}
						Merge
					</button>
				)}
				{!isLoading && pr?.status === "open" && dirty && (
					<button
						type="button"
						disabled={push.isPending}
						onClick={() => push.mutate()}
						className="flex items-center gap-1.5 px-2.5 py-1 rounded text-[11px] font-medium bg-btn text-text hover:bg-btn-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
					>
						{push.isPending ? (
							<Loader />
						) : (
							<ArrowUpFromLine size={10} />
						)}
						Push
					</button>
				)}
			</div>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function GitBarTabs() {
	const { workspace, diff } = useGitBar();
	const [activeTab, setActiveTab] = useState<"diff" | "checks">("diff");

	const prStatus = useQuery({
		queryKey: ["git_pr_status", workspace],
		queryFn: () => gitPrStatus(workspace),
		enabled: !!workspace,
		refetchInterval: 10000,
	});

	const hasPr = prStatus.data?.status === "open";

	const observation = useQuery({
		queryKey: ["git_pr_observe", workspace],
		queryFn: () => gitPrObserve(workspace),
		enabled: !!workspace && hasPr,
		refetchInterval: 15000,
	});

	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	const checksIndicator = (() => {
		if (!hasPr) return null;
		if (observation.isLoading) return <Loader />;
		const checks = observation.data?.checks ?? [];
		if (checks.length === 0) return null;
		const failStates: CheckState[] = ["failure", "startup_failure", "timed_out", "cancelled"];
		const pendingStates: CheckState[] = ["in_progress", "pending", "queued", "waiting", "requested"];
		if (checks.some((c) => failStates.includes(c.state)))
			return <X size={10} className="text-red-400" />;
		if (checks.some((c) => pendingStates.includes(c.state)))
			return <Loader />;
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
				{activeTab === "checks" && hasPr && <ChecksTab observation={observation.data ?? null} isLoading={observation.isLoading} />}
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
		<div className="flex flex-col">
			<DiffSectionView label="Local" section={diff.local} />
			<DiffSectionView label="Remote" section={diff.remote} />
		</div>
	);
}

function DiffSectionView({
	label,
	section,
}: { label: string; section: DiffSection }) {
	if (section.files.length === 0) return null;

	return (
		<div className="border-b border-border-light last:border-b-0">
			<div className="flex items-center justify-between px-3 py-1.5 text-[11px]">
				<span className="text-text-muted font-medium">{label}</span>
				<span className="text-text-muted">
					{section.overview.files_changed} file
					{section.overview.files_changed !== 1 ? "s" : ""}
					{section.overview.additions > 0 && (
						<span className="text-emerald-400">
							{" "}
							+{section.overview.additions}
						</span>
					)}
					{section.overview.deletions > 0 && (
						<span className="text-red-400">
							{" "}
							-{section.overview.deletions}
						</span>
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
			<span className="truncate min-w-0 text-text-muted">
				{file.path}
			</span>
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

function ChecksTab({ observation, isLoading }: { observation: import("../../lib/git").PullRequestObservation | null; isLoading: boolean }) {
	if (isLoading || !observation) {
		return (
			<div className="flex items-center justify-center py-8">
				<Loader />
			</div>
		);
	}

	const data = observation;

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
								const { openUrl } = await import(
									"@tauri-apps/plugin-opener"
								);
								openUrl(d.url);
							}}
							className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
						>
							{d.icon_url ? (
								<img src={d.icon_url} alt="" className="w-3.5 h-3.5 rounded shrink-0" />
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
					<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
						Checks
					</span>
					{data.checks.map((c) => (
						<button
							key={c.id}
							type="button"
							onClick={async () => {
								if (!c.link) return;
								const { openUrl } = await import(
									"@tauri-apps/plugin-opener"
								);
								openUrl(c.link);
							}}
							className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
						>
							<CheckStateIcon state={c.state} />
							<span className="text-text truncate">
								{c.name}
							</span>
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
		case "stale":
		case "unknown":
		default:
			return <Minus size={size} className="shrink-0 text-text-muted" />;
	}
}
