import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
	resolveForegroundPollInterval,
	usePageIsForeground,
} from "@/shared/lib/page-foreground";
import { Loader } from "@/shared/ui/loader";
import {
	type FileTreeDirectory,
	filesListDirectory,
	filesOpenInBrowser,
	filesPathOpensInBrowser,
} from "@/workspaces/files/api";
import { useFileSessions } from "@/workspaces/files/context";
import { FileIcon } from "@/workspaces/files/icons";
import { useGitSidebar } from "@/workspaces/git/context";
import { useWorkspaceRouteParams } from "@/workspaces/routes/params";
import { browserSessionHref, fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

interface TreeNode {
	children: Map<string, TreeNode>;
	expandable: boolean;
	gitIgnored: boolean;
	name: string;
	path: string;
	type: "directory" | "file";
}

type DiffInfo = { additions: number; deletions: number; status: string };
type DirStatus = {
	additions: number;
	deletions: number;
	fileCount: number;
	hasAdded: boolean;
	hasDeleted: boolean;
	hasModified: boolean;
};

export function GitFilesTab() {
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const { workspaceName: workspace, project } = useWorkspaceRouteParams();
	const workspaceSessions = useWorkspaceSessions();
	const { diff } = useGitSidebar();
	const { openFileTab } = useFileSessions();
	const isForeground = usePageIsForeground();
	const [expandedPaths, setExpandedPaths] = useState<Set<string>>(
		new Set([""]),
	);
	const rootQuery = useQuery({
		queryKey: ["files_list_directory", workspace, ""],
		queryFn: () => filesListDirectory(workspace),
		enabled: !!workspace,
		refetchInterval: resolveForegroundPollInterval({
			activeMs: 5000,
			enabled: !!workspace,
			hiddenMs: 30000,
			inactiveMs: 15000,
			isForeground,
		}),
	});
	const expandedDirectoryPaths = useMemo(
		() => Array.from(expandedPaths).filter((path) => path.length > 0).sort(),
		[expandedPaths],
	);
	const directoryQueries = useQueries({
		queries: expandedDirectoryPaths.map((path) => ({
			queryKey: ["files_list_directory", workspace, path],
			queryFn: () => filesListDirectory(workspace, path),
			enabled: !!workspace,
			refetchInterval: resolveForegroundPollInterval({
				activeMs: 5000,
				enabled: !!workspace,
				hiddenMs: 30000,
				inactiveMs: 15000,
				isForeground,
			}),
		})),
	});
	const diffByPath = useMemo(() => {
		const entries = new Map<string, DiffInfo>();
		for (const section of diff ? [diff.local, diff.remote] : []) {
			for (const file of section.files) {
				entries.set(file.path, {
					additions: file.additions,
					deletions: file.deletions,
					status: file.status,
				});
			}
		}
		return entries;
	}, [diff]);

	const dirStatusByPath = useMemo(() => {
		const statuses = new Map<string, DirStatus>();
		for (const [filePath, info] of diffByPath) {
			const parts = filePath.split("/");
			for (let i = 1; i < parts.length; i++) {
				const dirPath = parts.slice(0, i).join("/");
				const existing = statuses.get(dirPath) ?? {
					additions: 0,
					deletions: 0,
					fileCount: 0,
					hasAdded: false,
					hasDeleted: false,
					hasModified: false,
				};
				existing.additions += info.additions;
				existing.deletions += info.deletions;
				existing.fileCount += 1;
				if (info.status === "added") existing.hasAdded = true;
				else if (info.status === "deleted") existing.hasDeleted = true;
				else existing.hasModified = true;
				statuses.set(dirPath, existing);
			}
		}
		return statuses;
	}, [diffByPath]);

	const directorySlices = useMemo(() => {
		const slices = new Map<string, FileTreeDirectory>();
		if (rootQuery.data) {
			slices.set("", rootQuery.data);
		}
		for (const [index, path] of expandedDirectoryPaths.entries()) {
			const slice = directoryQueries[index]?.data;
			if (slice) {
				slices.set(path, slice);
			}
		}
		return slices;
	}, [directoryQueries, expandedDirectoryPaths, rootQuery.data]);

	const loadingPaths = useMemo(() => {
		const paths = new Set<string>();
		for (const [index, path] of expandedDirectoryPaths.entries()) {
			const query = directoryQueries[index];
			if (query && !query.data && (query.isLoading || query.isFetching)) {
				paths.add(path);
			}
		}
		return paths;
	}, [directoryQueries, expandedDirectoryPaths]);

	const errorsByPath = useMemo(() => {
		const errors = new Map<string, string>();
		for (const [index, path] of expandedDirectoryPaths.entries()) {
			const error = directoryQueries[index]?.error;
			if (error) {
				errors.set(path, queryErrorMessage(error));
			}
		}
		return errors;
	}, [directoryQueries, expandedDirectoryPaths]);

	const tree = useMemo(() => buildTree(directorySlices), [directorySlices]);

	if (rootQuery.isLoading && !rootQuery.data) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (rootQuery.isError && !rootQuery.data) {
		return (
			<div className="h-full flex items-center justify-center px-4 text-center text-sm text-text-muted">
				Failed to load files.
			</div>
		);
	}

	if (!rootQuery.data?.entries.length) {
		return (
			<div className="h-full flex items-center justify-center px-4 text-center text-sm text-text-muted">
				No files found
			</div>
		);
	}

	return (
		<div className="py-1">
			{Array.from(tree.children.values())
				.sort(compareTreeNodes)
				.map((node) => (
					<TreeRow
						key={node.path}
						depth={0}
						diffByPath={diffByPath}
						dirStatusByPath={dirStatusByPath}
						expandedPaths={expandedPaths}
						errorsByPath={errorsByPath}
						loadingPaths={loadingPaths}
						node={node}
						onFileOpen={async (path, persistent) => {
							if (filesPathOpensInBrowser(path)) {
								const result = await filesOpenInBrowser(workspace, path);
								navigate(
									browserSessionHref({
										project,
										workspace,
										attachmentId: result.attachment_id,
									}),
								);
								return;
							}
							const result = await openFileTab({
								path,
								persistent,
								workspace,
								workspaceSessions,
							});
							navigate(
								fileSessionHref({
									project,
									workspace,
									attachmentId: result.attachmentId,
								}),
							);
						}}
						onToggleFolder={(path) => {
							setExpandedPaths((previous) => {
								const next = new Set(previous);
								if (next.has(path)) {
									next.delete(path);
								} else {
									next.add(path);
									if (workspace && path) {
										void queryClient.prefetchQuery({
											queryKey: ["files_list_directory", workspace, path],
											queryFn: () => filesListDirectory(workspace, path),
										});
									}
								}
								return next;
							});
						}}
					/>
				))}
		</div>
	);
}

function TreeGuideLines({ depth }: { depth: number }) {
	if (depth === 0) return null;
	return (
		<>
			{Array.from({ length: depth }, (_, i) => {
				const left = i * 12 + 14;
				return (
					<span
						key={left}
						className="absolute top-0 bottom-0 w-px bg-border-light"
						style={{ left: `${left}px` }}
					/>
				);
			})}
		</>
	);
}

function TreeStatusRow({
	depth,
	message,
}: {
	depth: number;
	message: string;
}) {
	return (
		<div
			className="relative px-3 py-[3px] text-sm text-text-muted"
			style={{ paddingLeft: `${depth * 12 + 24}px` }}
		>
			<TreeGuideLines depth={depth} />
			{message}
		</div>
	);
}

function dirStatusType(status: DirStatus): "added" | "deleted" | "modified" {
	if (status.hasModified || (status.hasAdded && status.hasDeleted))
		return "modified";
	if (status.hasAdded) return "added";
	if (status.hasDeleted) return "deleted";
	return "modified";
}

function gitRowStyle(status: "added" | "deleted" | "modified" | null) {
	if (!status) return {};
	const colors = {
		added: {
			border: "rgba(52, 211, 153, 0.5)",
			bg: "rgba(52, 211, 153, 0.04)",
		},
		deleted: {
			border: "rgba(248, 113, 113, 0.5)",
			bg: "rgba(248, 113, 113, 0.04)",
		},
		modified: {
			border: "rgba(252, 211, 77, 0.5)",
			bg: "rgba(252, 211, 77, 0.04)",
		},
	};
	const { border, bg } = colors[status];
	return { boxShadow: `inset 2px 0 0 ${border}`, backgroundColor: bg };
}

function TreeRow({
	depth,
	diffByPath,
	dirStatusByPath,
	expandedPaths,
	errorsByPath,
	loadingPaths,
	node,
	onFileOpen,
	onToggleFolder,
}: {
	depth: number;
	diffByPath: Map<string, DiffInfo>;
	dirStatusByPath: Map<string, DirStatus>;
	expandedPaths: ReadonlySet<string>;
	errorsByPath: ReadonlyMap<string, string>;
	loadingPaths: ReadonlySet<string>;
	node: TreeNode;
	onFileOpen: (path: string, persistent: boolean) => Promise<void>;
	onToggleFolder: (path: string) => void;
}) {
	if (node.type === "directory") {
		const open = expandedPaths.has(node.path);
		const folderDiff = dirStatusByPath.get(node.path);
		const textClass = node.gitIgnored ? "text-text-muted" : "text-text";
		const loading = loadingPaths.has(node.path);
		const error = errorsByPath.get(node.path) ?? null;
		const children = Array.from(node.children.values()).sort(compareTreeNodes);

		return (
			<div>
				<button
					type="button"
					onClick={() => {
						if (node.expandable) {
							onToggleFolder(node.path);
						}
					}}
					className={`relative w-full flex items-center gap-1 px-3 py-[3px] text-sm ${textClass} hover:bg-btn-hover transition-colors`}
					style={{
						paddingLeft: `${depth * 12 + 8}px`,
						...gitRowStyle(folderDiff ? dirStatusType(folderDiff) : null),
					}}
				>
					<TreeGuideLines depth={depth} />
					<span className="w-4 flex items-center justify-center shrink-0">
						{node.expandable ? (
							loading ? (
								<Loader className="text-text-muted" />
							) : (
								<ChevronRight
									size={11}
									className={`text-text-muted transition-transform duration-150 ${open ? "rotate-90" : ""}`}
								/>
							)
						) : null}
					</span>
					<span className="truncate">{node.name}</span>
					{folderDiff && (
						<span className="shrink-0 ml-auto text-sm tabular-nums text-text-muted">
							{folderDiff.fileCount}
						</span>
					)}
				</button>
				{open && error && <TreeStatusRow depth={depth + 1} message="Failed to load directory." />}
				{open && !error && loading && children.length === 0 && (
					<TreeStatusRow depth={depth + 1} message="Loading..." />
				)}
				{open &&
					children.map((child) => (
						<TreeRow
							key={child.path}
							depth={depth + 1}
							diffByPath={diffByPath}
							dirStatusByPath={dirStatusByPath}
							expandedPaths={expandedPaths}
							errorsByPath={errorsByPath}
							loadingPaths={loadingPaths}
							node={child}
							onFileOpen={onFileOpen}
							onToggleFolder={onToggleFolder}
						/>
					))}
			</div>
		);
	}

	const diff = diffByPath.get(node.path) ?? null;
	const textClass = node.gitIgnored ? "text-text-muted" : "text-text";

	return (
		<button
			type="button"
			onClick={() => {
				void onFileOpen(node.path, false);
			}}
			onDoubleClick={() => {
				void onFileOpen(node.path, true);
			}}
			className={`relative w-full flex items-center justify-between gap-2 px-3 py-[3px] text-sm ${textClass} hover:bg-btn-hover transition-colors`}
			style={{
				paddingLeft: `${depth * 12 + 8}px`,
				...gitRowStyle(
					diff
						? diff.status === "added"
							? "added"
							: diff.status === "deleted"
								? "deleted"
								: "modified"
						: null,
				),
			}}
		>
			<TreeGuideLines depth={depth} />
			<span className="min-w-0 flex items-center gap-1">
				<span className="w-4 flex items-center justify-center shrink-0">
					<FileIcon path={node.path} size={12} />
				</span>
				<span className="truncate">{node.name}</span>
			</span>
			{diff && (
				<span className="shrink-0 flex items-center gap-1 text-sm">
					{diff.additions > 0 && (
						<span className="text-emerald-400/80">+{diff.additions}</span>
					)}
					{diff.deletions > 0 && (
						<span className="text-red-400/80">-{diff.deletions}</span>
					)}
				</span>
			)}
		</button>
	);
}

function buildTree(slices: ReadonlyMap<string, FileTreeDirectory>) {
	const root: TreeNode = {
		children: new Map(),
		expandable: true,
		gitIgnored: false,
		name: "",
		path: "",
		type: "directory",
	};
	const nodesByPath = new Map<string, TreeNode>([["", root]]);
	const directoryPaths = Array.from(slices.keys()).sort(compareDirectoryDepth);

	for (const directoryPath of directoryPaths) {
		const slice = slices.get(directoryPath);
		const parent = nodesByPath.get(directoryPath);
		if (!slice || !parent || parent.type !== "directory") {
			continue;
		}

		parent.children = new Map();
		for (const entry of slice.entries) {
			const child: TreeNode = {
				children: new Map(),
				expandable: entry.expandable,
				gitIgnored: entry.git_ignored,
				name: entry.name,
				path: entry.path,
				type: entry.kind,
			};
			parent.children.set(entry.name, child);
			nodesByPath.set(entry.path, child);
		}
	}

	return root;
}

function compareDirectoryDepth(left: string, right: string) {
	const leftDepth = left ? left.split("/").length : 0;
	const rightDepth = right ? right.split("/").length : 0;
	if (leftDepth !== rightDepth) {
		return leftDepth - rightDepth;
	}
	return left.localeCompare(right);
}

function compareTreeNodes(left: TreeNode, right: TreeNode) {
	if (left.type !== right.type) {
		return left.type === "directory" ? -1 : 1;
	}
	return left.name.localeCompare(right.name);
}

function queryErrorMessage(error: unknown) {
	if (error instanceof Error) {
		return error.message;
	}
	return String(error);
}
