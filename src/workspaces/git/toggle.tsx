import { PanelRight } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useGitSidebar } from "@/workspaces/git/context";
import { useWorkspaceReady } from "@/workspaces/state";

export function GitSidebarToggle() {
	const { isOpen, toggle, isInBranchWorkspace } = useGitSidebar();
	const isReady = useWorkspaceReady();

	if (isOpen || !isInBranchWorkspace || !isReady) return null;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					onClick={toggle}
					className="flex items-center justify-center h-5 px-1.5 rounded text-text-muted hover:bg-btn-hover hover:text-text-bright transition-colors"
				>
					<PanelRight size={12} />
				</button>
			</TooltipTrigger>
			<TooltipContent side="bottom">
				<span className="flex items-center gap-1.5">
					Toggle Git Sidebar
					<span className="flex items-center gap-0.5">
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text">
							⌘
						</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text">
							⌥
						</kbd>
						<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text">
							B
						</kbd>
					</span>
				</span>
			</TooltipContent>
		</Tooltip>
	);
}
