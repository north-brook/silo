import { PanelRight } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useGitSidebar } from "@/workspaces/git/context";

export function GitSidebarToggle() {
	const { isOpen, toggle, diff, isInBranchWorkspace } = useGitSidebar();

	if (isOpen || !isInBranchWorkspace) return null;

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
					Toggle Git Sidebar
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
