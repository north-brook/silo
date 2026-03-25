import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "@/shared/ui/toaster";
import {
	workspaceSessions as listWorkspaceSessions,
	type Workspace,
	type WorkspaceSession,
} from "@/workspaces/api";
import {
	filesCloseSession,
	filesGetWatchedState,
	filesOpenSession,
	filesSetWatchedPaths,
	type WatchedFileState,
} from "@/workspaces/files/api";
import {
	clearWorkspaceLocalSession,
	type DisplayWorkspaceSession,
	defaultFileTabState,
	type FileTabState,
	getWorkspaceLocalFileState,
	restoreWorkspaceLocalSession,
	updateWorkspaceLocalFileState,
	updateWorkspaceLocalSessionStates,
	updateWorkspaceLocalSessions,
	type WorkspaceLocalFileState,
	type WorkspaceLocalSessionSnapshot,
} from "@/workspaces/files/local-state";
import { useWorkspaceSessions, useWorkspaceState } from "@/workspaces/state";

export type {
	DisplayWorkspaceSession,
	FileTabState,
	WorkspaceLocalSessionSnapshot,
} from "@/workspaces/files/local-state";

interface OpenFileTabOptions {
	path: string;
	localFirst?: boolean;
	persistent: boolean;
	workspace: string;
	workspaceSessions: WorkspaceSession[];
}

interface FileSessionsContextValue {
	getDisplaySessions: (
		workspaceSessions: WorkspaceSession[],
	) => DisplayWorkspaceSession[];
	getSessionState: (attachmentId: string) => FileTabState;
	getWatchedFileState: (
		path: string | null | undefined,
	) => WatchedFileState | null;
	openFileTab: (
		options: OpenFileTabOptions,
	) => Promise<{ attachmentId: string; preview: boolean }>;
	promotePreviewTab: (
		workspace: string,
		workspaceSessions: WorkspaceSession[],
		attachmentId: string,
	) => Promise<string | null>;
	resolveSession: (
		workspaceSessions: WorkspaceSession[],
		attachmentId: string,
	) => DisplayWorkspaceSession | null;
	setSessionState: (
		attachmentId: string,
		next:
			| Partial<FileTabState>
			| ((previous: FileTabState) => Partial<FileTabState>),
	) => void;
	clearSession: (attachmentId: string) => void;
	restoreSession: (snapshot: WorkspaceLocalSessionSnapshot) => void;
}

const FileSessionsContext = createContext<FileSessionsContextValue | null>(
	null,
);

export function FileSessionsProvider({ children }: { children: ReactNode }) {
	const queryClient = useQueryClient();
	const { workspaceName } = useWorkspaceState();
	const workspaceSessions = useWorkspaceSessions();
	const [workspaceLocalFileStates, setWorkspaceLocalFileStates] = useState<
		Record<string, WorkspaceLocalFileState>
	>({});
	const workspaceLocalFileStatesRef = useRef(workspaceLocalFileStates);
	const pendingPersistentOpensRef = useRef(new Map<string, Set<string>>());
	const activeWorkspaceLocalFileState = useMemo(
		() => getWorkspaceLocalFileState(workspaceLocalFileStates, workspaceName),
		[workspaceLocalFileStates, workspaceName],
	);
	const localSessions = activeWorkspaceLocalFileState.sessions;
	const sessionStates = activeWorkspaceLocalFileState.sessionStates;

	useEffect(() => {
		workspaceLocalFileStatesRef.current = workspaceLocalFileStates;
	}, [workspaceLocalFileStates]);

	const updateWorkspaceSessionsState = useCallback(
		(
			workspace: string | null | undefined,
			updater: (
				previous: DisplayWorkspaceSession[],
			) => DisplayWorkspaceSession[],
		) => {
			setWorkspaceLocalFileStates((previous) =>
				updateWorkspaceLocalSessions(previous, workspace, updater),
			);
		},
		[],
	);

	const updateWorkspaceSessionTabStates = useCallback(
		(
			workspace: string | null | undefined,
			updater: (
				previous: Record<string, FileTabState>,
			) => Record<string, FileTabState>,
		) => {
			setWorkspaceLocalFileStates((previous) =>
				updateWorkspaceLocalSessionStates(previous, workspace, updater),
			);
		},
		[],
	);

	const clearPendingPersistentOpen = useCallback(
		(workspace: string | null | undefined, attachmentId: string) => {
			if (!workspace) {
				return;
			}
			const pendingWorkspaceOpens =
				pendingPersistentOpensRef.current.get(workspace);
			if (!pendingWorkspaceOpens) {
				return;
			}
			pendingWorkspaceOpens.delete(attachmentId);
			if (pendingWorkspaceOpens.size === 0) {
				pendingPersistentOpensRef.current.delete(workspace);
			}
		},
		[],
	);

	useEffect(() => {
		if (!workspaceName) {
			return;
		}
		const persistedIds = new Set(
			workspaceSessions
				.filter((session) => session.type === "file")
				.map((session) => session.attachment_id),
		);
		setWorkspaceLocalFileStates((previous) =>
			updateWorkspaceLocalFileState(previous, workspaceName, (current) => {
				const removedIds: string[] = [];
				const nextSessions = current.sessions.filter((session) => {
					if (!session.persistentAttachmentId) {
						return true;
					}
					const keep = persistedIds.has(session.persistentAttachmentId);
					if (!keep) {
						removedIds.push(session.attachment_id);
					}
					return keep;
				});
				if (removedIds.length === 0) {
					return current;
				}

				let nextSessionStates = current.sessionStates;
				for (const attachmentId of removedIds) {
					if (!(attachmentId in nextSessionStates)) {
						continue;
					}
					if (nextSessionStates === current.sessionStates) {
						nextSessionStates = { ...current.sessionStates };
					}
					delete nextSessionStates[attachmentId];
				}

				return {
					sessions: nextSessions,
					sessionStates: nextSessionStates,
				};
			}),
		);
	}, [workspaceName, workspaceSessions]);

	const setSessionState = useCallback<
		FileSessionsContextValue["setSessionState"]
	>(
		(attachmentId, next) => {
			updateWorkspaceSessionTabStates(workspaceName, (previous) => {
				const prior = previous[attachmentId] ?? defaultFileTabState;
				const patch = typeof next === "function" ? next(prior) : next;
				const updated = { ...prior, ...patch };
				if (
					updated.dirty === prior.dirty &&
					updated.conflicted === prior.conflicted &&
					updated.saving === prior.saving
				) {
					return previous;
				}
				return {
					...previous,
					[attachmentId]: updated,
				};
			});
		},
		[updateWorkspaceSessionTabStates, workspaceName],
	);

	const clearSession = useCallback(
		(attachmentId: string) => {
			clearPendingPersistentOpen(workspaceName, attachmentId);
			setWorkspaceLocalFileStates((previous) =>
				clearWorkspaceLocalSession(previous, workspaceName, attachmentId),
			);
		},
		[clearPendingPersistentOpen, workspaceName],
	);

	const restoreSession = useCallback(
		(snapshot: WorkspaceLocalSessionSnapshot) => {
			setWorkspaceLocalFileStates((previous) =>
				restoreWorkspaceLocalSession(previous, workspaceName, snapshot),
			);
		},
		[workspaceName],
	);

	const resolveSession = useCallback<
		FileSessionsContextValue["resolveSession"]
	>(
		(workspaceSessions, attachmentId) => {
			const localSession =
				localSessions.find(
					(session) => session.attachment_id === attachmentId,
				) ?? null;
			if (localSession) {
				return localSession;
			}
			return (
				workspaceSessions.find(
					(session) =>
						session.type === "file" && session.attachment_id === attachmentId,
				) ?? null
			);
		},
		[localSessions],
	);

	const getDisplaySessions = useCallback<
		FileSessionsContextValue["getDisplaySessions"]
	>(
		(workspaceSessions) => {
			if (localSessions.length === 0) {
				return workspaceSessions;
			}

			const shadowedPersistentIds = new Set(
				localSessions
					.map((session) => session.persistentAttachmentId)
					.filter((value): value is string => !!value),
			);

			const baseSessions = workspaceSessions.filter(
				(session) =>
					session.type !== "file" ||
					!shadowedPersistentIds.has(session.attachment_id),
			);

			return [...baseSessions, ...localSessions].sort((left, right) =>
				sessionSortKey(left.attachment_id).localeCompare(
					sessionSortKey(right.attachment_id),
				),
			);
		},
		[localSessions],
	);

	const displaySessions = useMemo(
		() => getDisplaySessions(workspaceSessions),
		[getDisplaySessions, workspaceSessions],
	);
	const watchedPaths = useMemo(
		() =>
			Array.from(
				new Set(
					displaySessions
						.filter((session) => session.type === "file")
						.map((session) => session.path?.trim() ?? "")
						.filter((path) => path.length > 0),
				),
			).sort(),
		[displaySessions],
	);
	useEffect(() => {
		if (!workspaceName) {
			return;
		}
		void filesSetWatchedPaths(workspaceName, watchedPaths).catch((error) => {
			console.error("failed to sync watched file paths", {
				workspace: workspaceName,
				error,
				paths: watchedPaths,
			});
		});
	}, [workspaceName, watchedPaths]);

	const watchedStateQuery = useQuery({
		queryKey: ["files_get_watched_state", workspaceName],
		queryFn: () => filesGetWatchedState(workspaceName),
		enabled: !!workspaceName && watchedPaths.length > 0,
		refetchInterval: 2000,
	});
	const watchedFilesByPath = useMemo(
		() =>
			new Map(
				(watchedStateQuery.data ?? []).map(
					(entry) => [entry.path, entry] as const,
				),
			),
		[watchedStateQuery.data],
	);

	const startPersistentOpenInBackground = useCallback(
		({
			attachmentId,
			path,
			workspace,
		}: {
			attachmentId: string;
			path: string;
			workspace: string;
		}) => {
			let pendingWorkspaceOpens =
				pendingPersistentOpensRef.current.get(workspace);
			if (!pendingWorkspaceOpens) {
				pendingWorkspaceOpens = new Set<string>();
				pendingPersistentOpensRef.current.set(workspace, pendingWorkspaceOpens);
			}
			if (pendingWorkspaceOpens.has(attachmentId)) {
				return;
			}
			pendingWorkspaceOpens.add(attachmentId);

			void (async () => {
				try {
					const result = await filesOpenSession(workspace, path);
					let sessionStillOpen = false;
					updateWorkspaceSessionsState(workspace, (previous) =>
						previous.map((session) => {
							if (session.attachment_id !== attachmentId) {
								return session;
							}
							sessionStillOpen = true;
							return {
								...session,
								persistentAttachmentId: result.attachment_id,
								preview: false,
							};
						}),
					);

					if (!sessionStillOpen) {
						const cachedWorkspace = queryClient.getQueryData<Workspace | null>([
							"workspaces_get_workspace",
							workspace,
						]);
						const cachedWorkspaceSessions = cachedWorkspace
							? listWorkspaceSessions(cachedWorkspace)
							: [];
						const hasReplacementSession =
							getWorkspaceLocalFileState(
								workspaceLocalFileStatesRef.current,
								workspace,
							).sessions.some(
								(session) =>
									session.type === "file" &&
									session.path === path &&
									(session.attachment_id !== attachmentId ||
										session.persistentAttachmentId === result.attachment_id),
							) ||
							cachedWorkspaceSessions.some(
								(session) =>
									session.type === "file" &&
									session.path === path &&
									session.attachment_id === result.attachment_id,
							);
						if (!hasReplacementSession) {
							void filesCloseSession(workspace, result.attachment_id).catch(
								() => {},
							);
						}
						return;
					}

					void queryClient.invalidateQueries({
						queryKey: ["workspaces_get_workspace", workspace],
					});
				} catch (error) {
					if (
						!getWorkspaceLocalFileState(
							workspaceLocalFileStatesRef.current,
							workspace,
						).sessions.some((session) => session.attachment_id === attachmentId)
					) {
						return;
					}
					toast({
						variant: "error",
						title: "File tab not persisted",
						description:
							error instanceof Error
								? error.message
								: "The file opened locally, but metadata sync failed.",
					});
				} finally {
					clearPendingPersistentOpen(workspace, attachmentId);
				}
			})();
		},
		[clearPendingPersistentOpen, queryClient, updateWorkspaceSessionsState],
	);

	const openFileTab = useCallback<FileSessionsContextValue["openFileTab"]>(
		async ({
			path,
			persistent,
			workspace,
			workspaceSessions,
			localFirst = false,
		}) => {
			const workspaceLocalSessions = getWorkspaceLocalFileState(
				workspaceLocalFileStates,
				workspace,
			).sessions;
			const localExisting = workspaceLocalSessions.find(
				(session) => session.path === path,
			);
			if (localExisting) {
				if (
					!persistent ||
					!localExisting.preview ||
					(localFirst && !localExisting.persistentAttachmentId)
				) {
					if (persistent && localFirst) {
						updateWorkspaceSessionsState(workspace, (previous) =>
							previous.map((session) =>
								session.attachment_id === localExisting.attachment_id
									? {
											...session,
											preview: false,
										}
									: session,
							),
						);
						if (!localExisting.persistentAttachmentId) {
							startPersistentOpenInBackground({
								attachmentId: localExisting.attachment_id,
								path,
								workspace,
							});
						}
					}
					return {
						attachmentId: localExisting.attachment_id,
						preview: persistent && localFirst ? false : !!localExisting.preview,
					};
				}

				const persistentAttachmentId = await ensurePersistentAttachmentId({
					localSession: localExisting,
					updateWorkspaceSessionsState,
					workspace,
					workspaceSessions,
				});
				if (persistentAttachmentId) {
					await queryClient.invalidateQueries({
						queryKey: ["workspaces_get_workspace", workspace],
					});
				}
				return {
					attachmentId: localExisting.attachment_id,
					preview: false,
				};
			}

			const existing = workspaceSessions.find(
				(session) => session.type === "file" && session.path === path,
			);
			if (existing) {
				return { attachmentId: existing.attachment_id, preview: false };
			}

			if (!persistent) {
				const previewAttachmentId =
					workspaceLocalSessions.find((session) => session.preview)
						?.attachment_id ?? createLocalAttachmentId();
				updateWorkspaceSessionsState(workspace, (previous) => {
					const previewSession = previous.find((session) => session.preview);
					if (previewSession) {
						return previous.map((session) =>
							session.attachment_id === previewSession.attachment_id
								? createLocalSession(path, previewSession.attachment_id, true)
								: session,
						);
					}
					return [
						...previous,
						createLocalSession(path, previewAttachmentId, true),
					];
				});
				updateWorkspaceSessionTabStates(workspace, (previous) => ({
					...previous,
					[previewAttachmentId]: defaultFileTabState,
				}));
				return { attachmentId: previewAttachmentId, preview: true };
			}

			if (localFirst) {
				const attachmentId = createLocalAttachmentId();
				updateWorkspaceSessionsState(workspace, (previous) => [
					...previous,
					createLocalSession(path, attachmentId, false),
				]);
				updateWorkspaceSessionTabStates(workspace, (previous) => ({
					...previous,
					[attachmentId]: defaultFileTabState,
				}));
				startPersistentOpenInBackground({
					attachmentId,
					path,
					workspace,
				});
				return { attachmentId, preview: false };
			}

			const result = await filesOpenSession(workspace, path);
			await queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			return { attachmentId: result.attachment_id, preview: false };
		},
		[
			queryClient,
			startPersistentOpenInBackground,
			updateWorkspaceSessionTabStates,
			updateWorkspaceSessionsState,
			workspaceLocalFileStates,
		],
	);

	const promotePreviewTab = useCallback<
		FileSessionsContextValue["promotePreviewTab"]
	>(
		async (workspace, workspaceSessions, attachmentId) => {
			const localSession = getWorkspaceLocalFileState(
				workspaceLocalFileStates,
				workspace,
			).sessions.find(
				(session) => session.attachment_id === attachmentId && session.preview,
			);
			if (!localSession?.path?.trim()) {
				return attachmentId;
			}

			const persistentAttachmentId = await ensurePersistentAttachmentId({
				localSession,
				updateWorkspaceSessionsState,
				workspace,
				workspaceSessions,
			});
			if (persistentAttachmentId) {
				await queryClient.invalidateQueries({
					queryKey: ["workspaces_get_workspace", workspace],
				});
			} else {
				updateWorkspaceSessionsState(workspace, (previous) =>
					previous.map((session) =>
						session.attachment_id === attachmentId
							? {
									...session,
									preview: false,
								}
							: session,
					),
				);
			}
			return attachmentId;
		},
		[queryClient, updateWorkspaceSessionsState, workspaceLocalFileStates],
	);

	const value = useMemo<FileSessionsContextValue>(
		() => ({
			getDisplaySessions,
			getSessionState: (attachmentId) =>
				sessionStates[attachmentId] ?? defaultFileTabState,
			getWatchedFileState: (path) =>
				path ? (watchedFilesByPath.get(path) ?? null) : null,
			openFileTab,
			promotePreviewTab,
			resolveSession,
			setSessionState,
			clearSession,
			restoreSession,
		}),
		[
			clearSession,
			getDisplaySessions,
			openFileTab,
			promotePreviewTab,
			resolveSession,
			restoreSession,
			sessionStates,
			setSessionState,
			watchedFilesByPath,
		],
	);

	return (
		<FileSessionsContext.Provider value={value}>
			{children}
		</FileSessionsContext.Provider>
	);
}

export function useFileSessions() {
	const context = useContext(FileSessionsContext);
	if (!context) {
		throw new Error(
			"useFileSessions must be used within a FileSessionsProvider",
		);
	}
	return context;
}

async function ensurePersistentAttachmentId({
	localSession,
	updateWorkspaceSessionsState,
	workspace,
	workspaceSessions,
}: {
	localSession: DisplayWorkspaceSession;
	updateWorkspaceSessionsState: (
		workspace: string,
		updater: (previous: DisplayWorkspaceSession[]) => DisplayWorkspaceSession[],
	) => void;
	workspace: string;
	workspaceSessions: WorkspaceSession[];
}) {
	if (localSession.persistentAttachmentId) {
		updateWorkspaceSessionsState(workspace, (previous) =>
			previous.map((session) =>
				session.attachment_id === localSession.attachment_id
					? {
							...session,
							preview: false,
						}
					: session,
			),
		);
		return localSession.persistentAttachmentId;
	}

	const existing = workspaceSessions.find(
		(session) => session.type === "file" && session.path === localSession.path,
	);
	if (existing) {
		updateWorkspaceSessionsState(workspace, (previous) =>
			previous.map((session) =>
				session.attachment_id === localSession.attachment_id
					? {
							...session,
							persistentAttachmentId: existing.attachment_id,
							preview: false,
						}
					: session,
			),
		);
		return existing.attachment_id;
	}

	const result = await filesOpenSession(workspace, localSession.path ?? "");
	updateWorkspaceSessionsState(workspace, (previous) =>
		previous.map((session) =>
			session.attachment_id === localSession.attachment_id
				? {
						...session,
						persistentAttachmentId: result.attachment_id,
						preview: false,
					}
				: session,
		),
	);
	return result.attachment_id;
}

function createLocalSession(
	path: string,
	attachmentId = createLocalAttachmentId(),
	preview = false,
): DisplayWorkspaceSession {
	return {
		type: "file",
		name: path.split("/").slice(-1)[0] || path,
		attachment_id: attachmentId,
		path,
		persistentAttachmentId: null,
		working: null,
		unread: null,
		preview,
	};
}

function createLocalAttachmentId() {
	return `file-${Date.now()}-local`;
}

function sessionSortKey(attachmentId: string) {
	const [, timestamp = "0"] = attachmentId.split("-", 2);
	return `${timestamp.padStart(20, "0")}:${attachmentId}`;
}
