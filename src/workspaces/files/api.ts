import { invoke } from "@/shared/lib/invoke";
import { filePathOpensInBrowser } from "@/workspaces/files/browser";

export interface FileTreeEntry {
	path: string;
	git_ignored: boolean;
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
			log: "state_changes_only",
			key: `poll:files_list_tree:${workspace}`,
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
			log: "state_changes_only",
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
	return invoke<void>("files_set_watched_paths", {
		workspace,
		paths,
	});
}

export function filesGetWatchedState(
	workspace: string,
): Promise<WatchedFileState[]> {
	return invoke<WatchedFileState[]>(
		"files_get_watched_state",
		{ workspace },
		{
			log: "state_changes_only",
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
