import { PatchDiff } from "@pierre/diffs/react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronsDownUp, ChevronsUpDown, ExternalLink } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
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
			--diffs-font-size: var(--text-sm);
			--diffs-line-height: 16px;
			--diffs-addition-color-override: #16a34a;
			--diffs-deletion-color-override: #f87171;
		}
		[data-diffs-header] {
			font-size: var(--text-sm);
			min-height: unset;
			padding-block: 6px;
			cursor: pointer;
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
			font-size: var(--text-sm);
		}
	`,
};

type DiffSectionKey = "local" | "remote";

type DiffRow =
	| {
			key: string;
			label: string;
			section: DiffSection;
			sectionKey: DiffSectionKey;
			type: "section";
	  }
	| {
			collapsed: boolean;
			file: DiffFile;
			key: string;
			sectionKey: DiffSectionKey;
			type: "file";
	  };

export function GitDiffTab() {
	const { diff, fullDiff, fullDiffLoading } = useGitSidebar();
	const scrollElementRef = useRef<HTMLDivElement | null>(null);
	const [collapsedByKey, setCollapsedByKey] = useState<Record<string, boolean>>(
		{},
	);
	const hasDiffFiles =
		(diff?.local.files.length ?? 0) > 0 || (diff?.remote.files.length ?? 0) > 0;
	const sections = useMemo(
		() =>
			fullDiff
				? [
						{
							sectionKey: "local" as const,
							label: "Local",
							section: fullDiff.local,
						},
						{
							sectionKey: "remote" as const,
							label: "Remote",
							section: fullDiff.remote,
						},
					].filter(({ section }) => section.files.length > 0)
				: [],
		[fullDiff],
	);
	const rows = useMemo<DiffRow[]>(
		() =>
			sections.flatMap(({ label, section, sectionKey }) => [
				{
					key: `${sectionKey}:section`,
					label,
					section,
					sectionKey,
					type: "section",
				} satisfies DiffRow,
				...section.files.map((file) => ({
					collapsed:
						collapsedByKey[diffFileKey(sectionKey, file.path)] ?? false,
					file,
					key: diffFileKey(sectionKey, file.path),
					sectionKey,
					type: "file" as const,
				})),
			]),
		[collapsedByKey, sections],
	);
	const sectionFiles = useMemo(
		() =>
			new Map(
				sections.map(({ sectionKey, section }) => [sectionKey, section.files] as const),
			),
		[sections],
	);
	const virtualizer = useVirtualizer({
		count: rows.length,
		estimateSize: (index) => estimateRowHeight(rows[index]),
		getScrollElement: () => scrollElementRef.current,
		overscan: 6,
	});

	if (fullDiffLoading && !fullDiff && hasDiffFiles) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (!diff || !hasDiffFiles) {
		return (
			<div className="flex h-full items-center justify-center px-6 text-center">
				<p className="text-sm text-text-muted">No changes yet</p>
			</div>
		);
	}

	return (
		<div ref={scrollElementRef} className="h-full overflow-y-auto bg-surface">
			<div
				className="relative px-2 pb-2"
				style={{ height: `${virtualizer.getTotalSize()}px` }}
			>
				{virtualizer.getVirtualItems().map((item) => {
					const row = rows[item.index];
					return (
						<div
							key={row.key}
							data-index={item.index}
							ref={virtualizer.measureElement}
							className="absolute left-0 top-0 w-full"
							style={{ transform: `translateY(${item.start}px)` }}
						>
							{row.type === "section" ? (
								<DiffSectionHeader
									allCollapsed={allFilesCollapsed(
										row.sectionKey,
										sectionFiles,
										collapsedByKey,
									)}
									label={row.label}
									onToggleAll={() => {
										const files = sectionFiles.get(row.sectionKey) ?? [];
										if (files.length === 0) {
											return;
										}
										const nextCollapsed = !allFilesCollapsed(
											row.sectionKey,
											sectionFiles,
											collapsedByKey,
										);
										setCollapsedByKey((previous) => {
											const next = { ...previous };
											for (const file of files) {
												next[diffFileKey(row.sectionKey, file.path)] =
													nextCollapsed;
											}
											return next;
										});
									}}
								/>
							) : (
								<DiffFileView
									collapsed={row.collapsed}
									file={row.file}
									onToggle={() => {
										setCollapsedByKey((previous) => ({
											...previous,
											[row.key]: !row.collapsed,
										}));
									}}
								/>
							)}
						</div>
					);
				})}
			</div>
		</div>
	);
}

function DiffSectionHeader({
	allCollapsed,
	label,
	onToggleAll,
}: {
	allCollapsed: boolean;
	label: string;
	onToggleAll: () => void;
}) {
	return (
		<div className="bg-surface flex items-center justify-between px-3 py-1.5 text-sm">
			<span className="text-sm text-text-muted font-medium uppercase tracking-wide">
				{label}
			</span>
			<button
				type="button"
				onClick={onToggleAll}
				className="text-text-muted hover:text-text transition-colors"
				title={allCollapsed ? "Expand all" : "Collapse all"}
			>
				{allCollapsed ? <ChevronsUpDown size={12} /> : <ChevronsDownUp size={12} />}
			</button>
		</div>
	);
}

function DiffFileView({
	collapsed,
	file,
	onToggle,
}: {
	collapsed: boolean;
	file: DiffFile;
	onToggle: () => void;
}) {
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
			<div className="flex items-center px-1 py-1 text-sm text-text-muted">
				<span className="truncate min-w-0">{file.path}</span>
				<span className="shrink-0 ml-auto text-text-placeholder italic">
					binary
				</span>
			</div>
		);
	}

	return (
		// biome-ignore lint/a11y/useSemanticElements: PatchDiff renders nested interactive controls, so this wrapper cannot be a button element.
		<div
			className="pierre-diff-container rounded border border-border-light"
			role="button"
			tabIndex={0}
			onClick={onToggle}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					onToggle();
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

function diffFileKey(sectionKey: DiffSectionKey, path: string) {
	return `${sectionKey}:${path}`;
}

function allFilesCollapsed(
	sectionKey: DiffSectionKey,
	sectionFiles: Map<DiffSectionKey, DiffFile[]>,
	collapsedByKey: Record<string, boolean>,
) {
	const files = sectionFiles.get(sectionKey) ?? [];
	return files.length > 0
		? files.every((file) => collapsedByKey[diffFileKey(sectionKey, file.path)] ?? false)
		: false;
}

function estimateRowHeight(row: DiffRow | undefined) {
	if (!row) {
		return 40;
	}
	if (row.type === "section") {
		return 34;
	}
	if (row.file.binary || row.file.patch == null) {
		return 36;
	}
	return row.collapsed ? 40 : 320;
}
