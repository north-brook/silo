import { useGitSidebar } from "@/workspaces/git/context";
import { GitSidebarHeader } from "@/workspaces/git/header";
import { GitSidebarTabs } from "@/workspaces/git/tabs";

export function GitSidebar() {
	const { isOpen, isInBranchWorkspace } = useGitSidebar();

	if (!isOpen || !isInBranchWorkspace) return null;

	return (
		<aside className="w-72 shrink-0 border-l border-border-light bg-bg flex flex-col min-h-0">
			<GitSidebarHeader />
			<GitSidebarTabs />
		</aside>
	);
}
