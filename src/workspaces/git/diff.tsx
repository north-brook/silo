import { useCallback, useMemo, useState } from "react";
import { ChevronsDownUp, ChevronsUpDown, ExternalLink } from "lucide-react";
import { PatchDiff } from "@pierre/diffs/react";
import { useNavigate } from "react-router-dom";
import { Loader } from "@/shared/ui/loader";
import { useFileSessions } from "@/workspaces/files/context";
import type { DiffFile, DiffSection } from "@/workspaces/git/api";
import { useGitSidebar } from "@/workspaces/git/context";
import { fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

const baseDiffOptions = {
	theme: "github-dark-default" as const,
	diffStyle: "unified" as const,
	diffIndicators: "bars" as const,
	disableLineNumbers: false,
	overflow: "wrap" as const,
	lineDiffType: "word" as const,
	disableBackground: true,
	expandUnchanged: false,
	hunkSeparators: "simple" as const,
	unsafeCSS: `
		:host {
			--diffs-header-font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', 'Cascadia Code', ui-monospace, monospace;
			--diffs-font-size: 11px;
			--diffs-line-height: 16px;
			--diffs-addition-color-override: #16a34a;
			--diffs-deletion-color-override: #f87171;
		}
		[data-diffs-header] {
			font-size: 11px;
			min-height: unset;
			padding-block: 6px;
			cursor: pointer;
			position: sticky;
			top: 26px;
			z-index: 3;
			background: var(--diffs-bg);
		}
		[data-change-icon] {
			display: none;
		}
		[data-diffs-header] [data-metadata] {
			gap: 8px;
		}
		[data-diffs-header] [data-additions-count],
		[data-diffs-header] [data-deletions-count] {
			font-size: 10px;
		}
	`,
};

export function GitDiffTab() {
	const { diff, diffLoading } = useGitSidebar();
	const hasDiffFiles =
		(diff?.local.files.length ?? 0) > 0 || (diff?.remote.files.length ?? 0) > 0;

	if (diffLoading && !diff) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (!diff || !hasDiffFiles) {
		return (
			<div className="flex h-full items-center justify-center px-6 text-center">
				<p className="text-sm text-text-placeholder">No changes to show.</p>
			</div>
		);
	}

	return (
		<div className="flex flex-col gap-2 pb-2">
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
	const [allCollapsed, setAllCollapsed] = useState(false);
	const [toggleGeneration, setToggleGeneration] = useState(0);

	if (section.files.length === 0) return null;

	return (
		<div>
			<div className="sticky top-0 z-10 bg-surface flex items-center justify-between px-3 py-1.5 text-[11px]">
				<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
					{label}
				</span>
				<button
					type="button"
					onClick={() => {
						setAllCollapsed((c) => !c);
						setToggleGeneration((g) => g + 1);
					}}
					className="text-text-muted hover:text-text transition-colors"
					title={allCollapsed ? "Expand all" : "Collapse all"}
				>
					{allCollapsed ? (
						<ChevronsUpDown size={12} />
					) : (
						<ChevronsDownUp size={12} />
					)}
				</button>
			</div>
			<div className="flex flex-col gap-2 px-2">
				{section.files.map((file) => (
					<DiffFileView
						key={file.path}
						file={file}
						forceCollapsed={allCollapsed}
						toggleGeneration={toggleGeneration}
					/>
				))}
			</div>
		</div>
	);
}

function DiffFileView({
	file,
	forceCollapsed,
	toggleGeneration,
}: {
	file: DiffFile;
	forceCollapsed: boolean;
	toggleGeneration: number;
}) {
	const [localCollapsed, setLocalCollapsed] = useState<{
		value: boolean;
		generation: number;
	}>({ value: false, generation: 0 });

	const collapsed =
		localCollapsed.generation >= toggleGeneration
			? localCollapsed.value
			: forceCollapsed;

	const navigate = useNavigate();
	const workspaceSessions = useWorkspaceSessions();
	const { openFileTab } = useFileSessions();
	const { workspace, project } = useGitSidebar();

	const options = useMemo(
		() => ({ ...baseDiffOptions, collapsed }),
		[collapsed],
	);

	const openFile = useCallback(
		(e: React.MouseEvent) => {
			e.stopPropagation();
			void openFileTab({
				localFirst: true,
				path: file.path,
				persistent: true,
				workspace,
				workspaceSessions,
			}).then((result) => {
				navigate(
					fileSessionHref({
						project,
						workspace,
						attachmentId: result.attachmentId,
					}),
				);
			});
		},
		[file.path, workspace, workspaceSessions, project, navigate, openFileTab],
	);

	const renderHeaderMetadata = useCallback(
		() => (
			<button
				type="button"
				onClick={openFile}
				className="inline-flex items-center text-text-muted hover:text-text transition-colors translate-y-px"
				title="Open file"
			>
				<ExternalLink size={12} />
			</button>
		),
		[openFile],
	);

	if (file.binary || file.patch == null) {
		return (
			<div className="flex items-center px-1 py-1 text-[11px] text-text-muted">
				<span className="truncate min-w-0">{file.path}</span>
				<span className="shrink-0 ml-auto text-text-placeholder italic">
					binary
				</span>
			</div>
		);
	}

	return (
		<div
			className="pierre-diff-container rounded border border-border-light"
			onClick={() =>
				setLocalCollapsed({
					value: !collapsed,
					generation: toggleGeneration + 1,
				})
			}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					setLocalCollapsed({
						value: !collapsed,
						generation: toggleGeneration + 1,
					});
				}
			}}
		>
			<PatchDiff
				patch={file.patch}
				options={options}
				renderHeaderMetadata={renderHeaderMetadata}
			/>
		</div>
	);
}
