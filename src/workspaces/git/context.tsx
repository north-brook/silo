import { useQuery } from "@tanstack/react-query";
import { createContext, type ReactNode, useContext, useState } from "react";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import { isTemplateWorkspace, workspaceIsReady } from "@/workspaces/api";
import {
	type Diff,
	gitDiff,
	gitPrSummary,
	type PullRequestSummary,
} from "@/workspaces/git/api";
import { useWorkspaceProject, useWorkspaceState } from "@/workspaces/state";

type GitSidebarTab = "diff" | "files" | "checks";

interface GitSidebarContextValue {
	isOpen: boolean;
	toggle: () => void;
	activeTab: GitSidebarTab;
	openTab: (tab: GitSidebarTab) => void;
	diff: Diff | null;
	diffLoading: boolean;
	hasChanges: boolean;
	workspace: string;
	project: string;
	isInBranchWorkspace: boolean;
	prSummary: PullRequestSummary | null;
	prSummaryLoading: boolean;
}

const GitSidebarContext = createContext<GitSidebarContextValue>({
	isOpen: false,
	toggle: () => {},
	activeTab: "diff",
	openTab: () => {},
	diff: null,
	diffLoading: false,
	hasChanges: false,
	workspace: "",
	project: "",
	isInBranchWorkspace: false,
	prSummary: null,
	prSummaryLoading: false,
});

export function useGitSidebar() {
	return useContext(GitSidebarContext);
}

export function GitSidebarProvider({ children }: { children: ReactNode }) {
	const { workspace, workspaceName } = useWorkspaceState();
	const project = useWorkspaceProject();

	const isInBranchWorkspace =
		!!workspaceName && !!workspace && !isTemplateWorkspace(workspace);
	const isReadyBranchWorkspace =
		isInBranchWorkspace && !!workspace && workspaceIsReady(workspace);

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

	const prSummaryQuery = useQuery({
		queryKey: ["git_pr_summary", workspaceName],
		queryFn: () => gitPrSummary(workspaceName),
		enabled: isReadyBranchWorkspace,
		refetchInterval: 10000,
	});

	const hasPr = prSummaryQuery.data?.status === "open";

	const [isOpen, setIsOpen] = useState(false);
	const [activeTab, setActiveTab] = useState<GitSidebarTab>("diff");
	const visibleTab = hasPr
		? activeTab
		: activeTab === "checks"
			? "diff"
			: activeTab;

	useShortcut<void>({
		event: shortcutEvents.toggleGitBar,
		onTrigger: () => {
			setIsOpen((open) => !open);
		},
		onKeyDown: (e) => {
			if (e.metaKey && e.altKey && !e.shiftKey && e.key.toLowerCase() === "b") {
				e.preventDefault();
				setIsOpen((open) => !open);
			}
		},
	});

	const openTab = (tab: GitSidebarTab) => {
		if (tab === "checks" && !hasPr) {
			return;
		}

		setActiveTab(tab);
		setIsOpen(true);
	};

	useShortcut<void>({
		event: shortcutEvents.openGitDiff,
		onTrigger: () => {
			openTab("diff");
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey || e.altKey || e.ctrlKey) return;
			if (e.key.toLowerCase() === "d") {
				e.preventDefault();
				openTab("diff");
			}
		},
	});

	useShortcut<void>({
		event: shortcutEvents.openGitFiles,
		onTrigger: () => {
			openTab("files");
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey || e.altKey || e.ctrlKey) return;
			if (e.key.toLowerCase() === "e") {
				e.preventDefault();
				openTab("files");
			}
		},
	});

	useShortcut<void>({
		event: shortcutEvents.openGitChecks,
		onTrigger: () => {
			openTab("checks");
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey || e.altKey || e.ctrlKey) return;
			if (e.key.toLowerCase() === "c") {
				e.preventDefault();
				openTab("checks");
			}
		},
	});

	return (
		<GitSidebarContext.Provider
			value={{
				isOpen,
				toggle: () => setIsOpen((open) => !open),
				activeTab: visibleTab,
				openTab,
				diff: diff.data ?? null,
				diffLoading: diff.isLoading,
				hasChanges,
				workspace: workspaceName,
				project,
				isInBranchWorkspace,
				prSummary: prSummaryQuery.data ?? null,
				prSummaryLoading: prSummaryQuery.isLoading,
			}}
		>
			{children}
		</GitSidebarContext.Provider>
	);
}
