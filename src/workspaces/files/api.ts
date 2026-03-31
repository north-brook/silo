import { hotPollLogMode, invoke } from "@/shared/lib/invoke";
import { filePathOpensInBrowser } from "@/workspaces/files/browser";

export interface FileTreeEntry {
	path: string;
	git_ignored: boolean;
}

export type FileTreeNodeKind = "file" | "directory";

export interface FileTreeNode {
	path: string;
	name: string;
	kind: FileTreeNodeKind;
	git_ignored: boolean;
	expandable: boolean;
}

export interface FileTreeDirectory {
	directory_path: string;
	entries: FileTreeNode[];
}

export interface FileReadResult {
	path: string;
	exists: boolean;
	binary: boolean;
	revision: string;
	content: string | null;
}

export type FileSaveStatus = "saved" | "conflict" | "missing";

export interface FileSaveResult {
	status: FileSaveStatus;
	revision: string | null;
}

export interface FileSessionResult {
	attachment_id: string;
}

export interface FileBrowserSessionResult {
	attachment_id: string;
}

export interface WatchedFileState {
	path: string;
	exists: boolean;
	binary: boolean;
	revision: string;
}

export function filesListTree(workspace: string): Promise<FileTreeEntry[]> {
	return invoke<FileTreeEntry[]>(
		"files_list_tree",
		{ workspace },
		{
			log: hotPollLogMode(),
			key: `poll:files_list_tree:${workspace}`,
		},
	);
}

export function filesListDirectory(
	workspace: string,
	path?: string | null,
): Promise<FileTreeDirectory> {
	return invoke<FileTreeDirectory>(
		"files_list_directory",
		path ? { workspace, path } : { workspace },
		{
			log: hotPollLogMode(),
			key: `poll:files_list_directory:${workspace}:${path ?? ""}`,
		},
	);
}

export function filesRead(
	workspace: string,
	path: string,
): Promise<FileReadResult> {
	return invoke<FileReadResult>(
		"files_read",
		{ workspace, path },
		{
			log: hotPollLogMode(),
			key: `poll:files_read:${workspace}:${path}`,
		},
	);
}

export function filesSave(
	workspace: string,
	path: string,
	content: string,
	baseRevision: string,
): Promise<FileSaveResult> {
	return invoke<FileSaveResult>("files_save", {
		workspace,
		path,
		content,
		baseRevision,
	});
}

export function filesSetWatchedPaths(
	workspace: string,
	paths: string[],
): Promise<void> {
	return invoke<void>(
		"files_set_watched_paths",
		{
			workspace,
			paths,
		},
		{
			log: "errors_only",
		},
	);
}

export function filesGetWatchedState(
	workspace: string,
): Promise<WatchedFileState[]> {
	return invoke<WatchedFileState[]>(
		"files_get_watched_state",
		{ workspace },
		{
			log: hotPollLogMode(),
			key: `poll:files_get_watched_state:${workspace}`,
		},
	);
}

export function filesOpenSession(
	workspace: string,
	path: string,
): Promise<FileSessionResult> {
	return invoke<FileSessionResult>("files_open_session", { workspace, path });
}

export function filesPathOpensInBrowser(path: string): boolean {
	return filePathOpensInBrowser(path);
}

export function filesOpenInBrowser(
	workspace: string,
	path: string,
): Promise<FileBrowserSessionResult> {
	return invoke<FileBrowserSessionResult>("browser_open_workspace_file", {
		workspace,
		path,
	});
}

export function filesCloseSession(
	workspace: string,
	attachmentId: string,
): Promise<void> {
	return invoke<void>("files_close_session", { workspace, attachmentId });
}
