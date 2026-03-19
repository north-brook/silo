import { useQuery } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Loader } from "@/shared/ui/loader";
import { type FileTreeEntry, filesListTree } from "@/workspaces/files/api";
import { useFileSessions } from "@/workspaces/files/context";
import {
	FolderClosedIcon,
	FolderOpenIcon,
	fileIconForPath,
} from "@/workspaces/files/icons";
import { useGitSidebar } from "@/workspaces/git/context";
import { useWorkspaceRouteParams } from "@/workspaces/routes/params";
import { fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

interface TreeNode {
	children: Map<string, TreeNode>;
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
	const { workspaceName: workspace, project } = useWorkspaceRouteParams();
	const workspaceSessions = useWorkspaceSessions();
	const { diff } = useGitSidebar();
	const { openFileTab } = useFileSessions();
	const filesQuery = useQuery({
		queryKey: ["files_list_tree", workspace],
		queryFn: () => filesListTree(workspace),
		enabled: !!workspace,
		refetchInterval: 5000,
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

	const tree = useMemo(
		() => buildTree(filesQuery.data ?? []),
		[filesQuery.data],
	);
	const [expandedPaths, setExpandedPaths] = useState<Set<string>>(
		new Set([""]),
	);

	if (filesQuery.isLoading && !filesQuery.data) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (!filesQuery.data?.length) {
		return (
			<div className="h-full flex items-center justify-center px-4 text-center text-[11px] text-text-muted">
				No tracked or untracked files found.
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
						node={node}
						onFileOpen={async (path, persistent) => {
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
			{Array.from({ length: depth }, (_, i) => (
				<span
					key={i}
					className="absolute top-0 bottom-0 w-px bg-border-light"
					style={{ left: `${i * 12 + 14}px` }}
				/>
			))}
		</>
	);
}

function dirStatusType(
	status: DirStatus,
): "added" | "deleted" | "modified" {
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
	node,
	onFileOpen,
	onToggleFolder,
}: {
	depth: number;
	diffByPath: Map<string, DiffInfo>;
	dirStatusByPath: Map<string, DirStatus>;
	expandedPaths: ReadonlySet<string>;
	node: TreeNode;
	onFileOpen: (path: string, persistent: boolean) => Promise<void>;
	onToggleFolder: (path: string) => void;
}) {
	if (node.type === "directory") {
		const open = expandedPaths.has(node.path);
		const FolderIcon = open ? FolderOpenIcon : FolderClosedIcon;
		const folderDiff = dirStatusByPath.get(node.path);

		return (
			<div>
				<button
					type="button"
					onClick={() => onToggleFolder(node.path)}
					className="relative w-full flex items-center gap-1.5 px-3 py-[3px] text-[11px] text-text hover:bg-btn-hover transition-colors"
					style={{
						paddingLeft: `${depth * 12 + 8}px`,
						...gitRowStyle(
							folderDiff ? dirStatusType(folderDiff) : null,
						),
					}}
				>
					<TreeGuideLines depth={depth} />
					<ChevronRight
						size={11}
						className={`shrink-0 text-text-muted transition-transform duration-150 ${open ? "rotate-90" : ""}`}
					/>
					<FolderIcon
						size={12}
						className="shrink-0 text-text-muted"
					/>
					<span className="truncate">{node.name}</span>
					{folderDiff && (
						<span className="shrink-0 ml-auto text-[10px] tabular-nums text-text-muted">
							{folderDiff.fileCount}
						</span>
					)}
				</button>
				{open &&
					Array.from(node.children.values())
						.sort(compareTreeNodes)
						.map((child) => (
							<TreeRow
								key={child.path}
								depth={depth + 1}
								diffByPath={diffByPath}
								dirStatusByPath={dirStatusByPath}
								expandedPaths={expandedPaths}
								node={child}
								onFileOpen={onFileOpen}
								onToggleFolder={onToggleFolder}
							/>
						))}
			</div>
		);
	}

	const Icon = fileIconForPath(node.path);
	const diff = diffByPath.get(node.path) ?? null;

	return (
		<button
			type="button"
			onClick={() => {
				void onFileOpen(node.path, false);
			}}
			onDoubleClick={() => {
				void onFileOpen(node.path, true);
			}}
			className="relative w-full flex items-center justify-between gap-2 px-3 py-[3px] text-[11px] text-text hover:bg-btn-hover transition-colors"
			style={{
				paddingLeft: `${depth * 12 + 25}px`,
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
			<span className="min-w-0 flex items-center gap-1.5">
				<Icon size={12} className="shrink-0 text-text-muted" />
				<span className="truncate">{node.name}</span>
			</span>
			{diff && (
				<span className="shrink-0 flex items-center gap-1 text-[10px]">
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

function buildTree(entries: FileTreeEntry[]) {
	const root: TreeNode = {
		children: new Map(),
		name: "",
		path: "",
		type: "directory",
	};

	for (const entry of entries) {
		const segments = entry.path.split("/").filter(Boolean);
		let current = root;
		let currentPath = "";
		for (const [index, segment] of segments.entries()) {
			currentPath = currentPath ? `${currentPath}/${segment}` : segment;
			const isLeaf = index === segments.length - 1;
			const existing = current.children.get(segment);
			if (existing) {
				current = existing;
				continue;
			}

			const next: TreeNode = {
				children: new Map(),
				name: segment,
				path: currentPath,
				type: isLeaf ? "file" : "directory",
			};
			current.children.set(segment, next);
			current = next;
		}
	}

	return root;
}

function compareTreeNodes(left: TreeNode, right: TreeNode) {
	if (left.type !== right.type) {
		return left.type === "directory" ? -1 : 1;
	}
	return left.name.localeCompare(right.name);
}
