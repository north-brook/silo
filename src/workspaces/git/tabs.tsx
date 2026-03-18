import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { GitFilesTab } from "@/workspaces/files/explorer";
import {
	GitChecksStatusIndicator,
	GitChecksTab,
} from "@/workspaces/git/checks";
import { useGitSidebar } from "@/workspaces/git/context";
import { GitDiffTab } from "@/workspaces/git/diff";

export function GitSidebarTabs() {
	const {
		activeTab,
		diff,
		hasChanges,
		openTab,
		prStatus,
		observation,
		observationLoading,
	} = useGitSidebar();

	const hasPr = prStatus?.status === "open";
	const additions = diff?.overview.additions ?? 0;
	const deletions = diff?.overview.deletions ?? 0;

	return (
		<>
			<div className="w-full bg-bg shrink-0 flex items-end">
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={() => openTab("files")}
							className={`h-9 flex items-center gap-1.5 px-3 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
								activeTab === "files"
									? "bg-surface text-text-bright border-r-border-light border-b-surface"
									: "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
							}`}
						>
							Files
						</button>
					</TooltipTrigger>
					<TooltipContent side="bottom">
						<HotkeyHint keys={["⌘", "⇧", "E"]} />
					</TooltipContent>
				</Tooltip>
				<Tooltip>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={() => openTab("diff")}
							className={`h-9 flex items-center gap-1.5 px-3 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
								activeTab === "diff"
									? "bg-surface text-text-bright border-r-border-light border-b-surface"
									: hasChanges
										? "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
										: "text-text-placeholder border-r-border-light border-b-border-light cursor-default"
							}`}
							disabled={!hasChanges}
						>
							Diff
							<span className="text-emerald-400">+{additions}</span>
							<span className="text-red-400">-{deletions}</span>
						</button>
					</TooltipTrigger>
					<TooltipContent side="bottom">
						<HotkeyHint keys={["⌘", "⇧", "D"]} />
					</TooltipContent>
				</Tooltip>
				{hasPr && (
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={() => openTab("checks")}
								className={`h-9 flex items-center gap-1.5 px-3 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
									activeTab === "checks"
										? "bg-surface text-text-bright border-r-border-light border-b-surface"
										: "text-text-muted border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text"
								}`}
							>
								Checks
								<GitChecksStatusIndicator
									observation={observation}
									isLoading={observationLoading}
								/>
							</button>
						</TooltipTrigger>
						<TooltipContent side="bottom">
							<HotkeyHint keys={["⌘", "⇧", "C"]} />
						</TooltipContent>
					</Tooltip>
				)}
				<div className="flex-1 h-9 border-b border-border-light" />
			</div>
			<div className="flex-1 overflow-y-auto bg-surface">
				{activeTab === "files" && <GitFilesTab />}
				{activeTab === "diff" && <GitDiffTab />}
				{activeTab === "checks" && hasPr && (
					<GitChecksTab
						observation={observation}
						isLoading={observationLoading}
					/>
				)}
			</div>
		</>
	);
}

function HotkeyHint({ keys }: { keys: string[] }) {
	return (
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
	);
}
