import { useNavigate } from "react-router-dom";
import { useFileSessions } from "@/workspaces/files/context";
import type { DiffFile, DiffSection } from "@/workspaces/git/api";
import { useGitSidebar } from "@/workspaces/git/context";
import { fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

export function GitDiffTab() {
	const { diff } = useGitSidebar();

	if (!diff) return null;

	return (
		<div className="flex flex-col gap-2">
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
	if (section.files.length === 0) return null;

	return (
		<div>
			<div className="flex items-center px-3 py-1.5 text-[11px]">
				<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
					{label}
				</span>
			</div>
			{section.files.map((file) => (
				<DiffFileRow key={file.path} file={file} />
			))}
		</div>
	);
}

function DiffFileRow({ file }: { file: DiffFile }) {
	const navigate = useNavigate();
	const workspaceSessions = useWorkspaceSessions();
	const { openFileTab } = useFileSessions();
	const { workspace, project } = useGitSidebar();

	return (
		<button
			type="button"
			onClick={() => {
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
			}}
			className="w-full flex items-center justify-between px-3 py-1 text-[11px] hover:bg-btn-hover transition-colors text-left"
		>
			<span className="truncate min-w-0 text-text">{file.path}</span>
			<span className="shrink-0 ml-2 text-text-muted">
				{file.additions > 0 && (
					<span className="text-emerald-400">+{file.additions}</span>
				)}
				{file.additions > 0 && file.deletions > 0 && " "}
				{file.deletions > 0 && (
					<span className="text-red-400">-{file.deletions}</span>
				)}
			</span>
		</button>
	);
}
