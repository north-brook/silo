import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	createContext,
	type Dispatch,
	type ReactNode,
	type SetStateAction,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "@/shared/ui/toaster";
import type { WorkspaceSession } from "@/workspaces/api";
import {
	filesGetWatchedState,
	filesCloseSession,
	filesOpenSession,
	filesSetWatchedPaths,
	type WatchedFileState,
} from "@/workspaces/files/api";
import { useWorkspaceSessions, useWorkspaceState } from "@/workspaces/state";

export interface FileTabState {
	conflicted: boolean;
	dirty: boolean;
	saving: boolean;
}

export interface DisplayWorkspaceSession extends WorkspaceSession {
	persistentAttachmentId?: string | null;
	preview?: boolean;
}

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
}

const defaultTabState: FileTabState = {
	conflicted: false,
	dirty: false,
	saving: false,
};

const FileSessionsContext = createContext<FileSessionsContextValue | null>(
	null,
);

export function FileSessionsProvider({ children }: { children: ReactNode }) {
	const queryClient = useQueryClient();
	const { workspaceName } = useWorkspaceState();
	const workspaceSessions = useWorkspaceSessions();
	const [localSessions, setLocalSessions] = useState<DisplayWorkspaceSession[]>(
		[],
	);
	const [sessionStates, setSessionStates] = useState<
		Record<string, FileTabState>
	>({});
	const localSessionsRef = useRef(localSessions);
	const workspaceSessionsRef = useRef(workspaceSessions);
	const pendingPersistentOpensRef = useRef(
		new Map<string, { path: string; workspace: string }>(),
	);

	useEffect(() => {
		localSessionsRef.current = localSessions;
	}, [localSessions]);

	useEffect(() => {
		workspaceSessionsRef.current = workspaceSessions;
	}, [workspaceSessions]);

	const setSessionState = useCallback<
		FileSessionsContextValue["setSessionState"]
	>((attachmentId, next) => {
		setSessionStates((previous) => {
			const prior = previous[attachmentId] ?? defaultTabState;
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
	}, []);

	const clearSession = useCallback((attachmentId: string) => {
		pendingPersistentOpensRef.current.delete(attachmentId);
		setLocalSessions((current) =>
			current.filter((session) => session.attachment_id !== attachmentId),
		);
		setSessionStates((previous) => {
			if (!(attachmentId in previous)) {
				return previous;
			}
			const next = { ...previous };
			delete next[attachmentId];
			return next;
		});
	}, []);

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
			const pending = pendingPersistentOpensRef.current;
			if (pending.has(attachmentId)) {
				return;
			}
			pending.set(attachmentId, { path, workspace });

			void (async () => {
				try {
					const result = await filesOpenSession(workspace, path);
					let sessionStillOpen = false;
					setLocalSessions((previous) =>
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
						const hasReplacementSession =
							localSessionsRef.current.some(
								(session) =>
									session.type === "file" &&
									session.path === path &&
									(session.attachment_id !== attachmentId ||
										session.persistentAttachmentId === result.attachment_id),
							) ||
							workspaceSessionsRef.current.some(
								(session) =>
									session.type === "file" &&
									session.path === path &&
									session.attachment_id === result.attachment_id,
							);
						if (!hasReplacementSession) {
							void filesCloseSession(
								workspace,
								result.attachment_id,
							).catch(() => {});
						}
						return;
					}

					void queryClient.invalidateQueries({
						queryKey: ["workspaces_get_workspace", workspace],
					});
				} catch (error) {
					if (!localSessionsRef.current.some(
						(session) => session.attachment_id === attachmentId,
					)) {
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
					pending.delete(attachmentId);
				}
			})();
		},
		[queryClient],
	);

	const openFileTab = useCallback<FileSessionsContextValue["openFileTab"]>(
		async ({
			path,
			persistent,
			workspace,
			workspaceSessions,
			localFirst = false,
		}) => {
			const localExisting = localSessions.find(
				(session) => session.path === path,
			);
			if (localExisting) {
				if (
					!persistent ||
					!localExisting.preview ||
					(localFirst && !localExisting.persistentAttachmentId)
				) {
					if (persistent && localFirst) {
						setLocalSessions((previous) =>
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
						preview:
							persistent && localFirst
								? false
								: !!localExisting.preview,
					};
				}

				const persistentAttachmentId = await ensurePersistentAttachmentId({
					localSession: localExisting,
					setLocalSessions,
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
					localSessions.find((session) => session.preview)?.attachment_id ??
					createLocalAttachmentId();
				setLocalSessions((previous) => {
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
				setSessionStates((previous) => ({
					...previous,
					[previewAttachmentId]: defaultTabState,
				}));
				return { attachmentId: previewAttachmentId, preview: true };
			}

			if (localFirst) {
				const attachmentId = createLocalAttachmentId();
				setLocalSessions((previous) => [
					...previous,
					createLocalSession(path, attachmentId, false),
				]);
				setSessionStates((previous) => ({
					...previous,
					[attachmentId]: defaultTabState,
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
		[localSessions, queryClient, startPersistentOpenInBackground],
	);

	const promotePreviewTab = useCallback<
		FileSessionsContextValue["promotePreviewTab"]
	>(
		async (workspace, workspaceSessions, attachmentId) => {
			const localSession = localSessions.find(
				(session) => session.attachment_id === attachmentId && session.preview,
			);
			if (!localSession?.path?.trim()) {
				return attachmentId;
			}

			const persistentAttachmentId = await ensurePersistentAttachmentId({
				localSession,
				setLocalSessions,
				workspace,
				workspaceSessions,
			});
			if (persistentAttachmentId) {
				await queryClient.invalidateQueries({
					queryKey: ["workspaces_get_workspace", workspace],
				});
			} else {
				setLocalSessions((previous) =>
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
		[localSessions, queryClient],
	);

	const value = useMemo<FileSessionsContextValue>(
		() => ({
			getDisplaySessions,
			getSessionState: (attachmentId) =>
				sessionStates[attachmentId] ?? defaultTabState,
			getWatchedFileState: (path) =>
				path ? (watchedFilesByPath.get(path) ?? null) : null,
			openFileTab,
			promotePreviewTab,
			resolveSession,
			setSessionState,
			clearSession,
		}),
		[
			clearSession,
			getDisplaySessions,
			openFileTab,
			promotePreviewTab,
			resolveSession,
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
	setLocalSessions,
	workspace,
	workspaceSessions,
}: {
	localSession: DisplayWorkspaceSession;
	setLocalSessions: Dispatch<SetStateAction<DisplayWorkspaceSession[]>>;
	workspace: string;
	workspaceSessions: WorkspaceSession[];
}) {
	if (localSession.persistentAttachmentId) {
		setLocalSessions((previous) =>
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
		setLocalSessions((previous) =>
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
	setLocalSessions((previous) =>
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
