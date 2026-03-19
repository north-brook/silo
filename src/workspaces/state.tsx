import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
} from "react";
import {
	type CloudSession,
	normalizeWorkspaceSession,
} from "@/workspaces/hosts/model";
import { useWorkspaceRouteParams } from "@/workspaces/routes/params";
import { invoke } from "@/shared/lib/invoke";
import {
	isTemplateWorkspace,
	type Workspace,
	type WorkspaceActiveSession,
	type WorkspaceSession,
	workspaceIsReady,
	workspaceSessions,
} from "@/workspaces/api";

interface WorkspaceStateContextValue {
	workspaceName: string;
	routeProject: string;
	workspace: Workspace | null;
	isLoading: boolean;
	isMissing: boolean;
	invalidateWorkspace: () => void;
}

const WorkspaceStateContext = createContext<WorkspaceStateContextValue>({
	workspaceName: "",
	routeProject: "",
	workspace: null,
	isLoading: false,
	isMissing: false,
	invalidateWorkspace: () => {},
});

interface WorkspaceStateEventPayload {
	workspace: string;
	clearedActiveSession: boolean;
	removedSessionAttachmentId?: string | null;
	removedSessionKind?: string | null;
}

function WorkspaceStateProviderInner({
	children,
	projectParam,
	workspaceName,
}: {
	children: ReactNode;
	projectParam: string;
	workspaceName: string;
}) {
	const queryClient = useQueryClient();

	const workspaceQuery = useQuery({
		queryKey: ["workspaces_get_workspace", workspaceName],
		queryFn: () =>
			invoke<Workspace | null>(
				"workspaces_get_workspace",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:workspaces_get_workspace:${workspaceName}`,
				},
			),
		enabled: !!workspaceName,
		refetchInterval: 2000,
	});

	const invalidateWorkspace = useCallback(() => {
		if (!workspaceName) {
			return;
		}
		queryClient.invalidateQueries({
			queryKey: ["workspaces_get_workspace", workspaceName],
		});
	}, [queryClient, workspaceName]);

	const applyWorkspaceStateEvent = useCallback(
		(
			current: Workspace | null | undefined,
			event: WorkspaceStateEventPayload,
		): Workspace | null | undefined => {
			if (!current) {
				return current;
			}

			const nextActiveSession: WorkspaceActiveSession | null | undefined =
				event.clearedActiveSession
					? null
					: current.active_session ?? null;

			if (
				!event.removedSessionAttachmentId ||
				!event.removedSessionKind
			) {
				if (nextActiveSession === current.active_session) {
					return current;
				}
				return {
					...current,
					active_session: nextActiveSession,
				};
			}

			const removeSession = (sessions: WorkspaceSession[]) =>
				sessions.filter(
					(session) =>
						!(
							session.type === event.removedSessionKind &&
							session.attachment_id === event.removedSessionAttachmentId
						),
				);

			const nextTerminals = removeSession(current.terminals);
			const nextBrowsers = removeSession(current.browsers);
			const nextFiles = removeSession(current.files);
			const assistantPresent = nextTerminals.some(
				(session) => session.working != null || session.unread != null,
			);
			const nextWorking = assistantPresent
				? nextTerminals.some((session) => session.working === true)
				: null;
			const nextUnread = nextTerminals.some(
				(session) => session.unread === true,
			);

			if (
				nextTerminals.length === current.terminals.length &&
				nextBrowsers.length === current.browsers.length &&
				nextFiles.length === current.files.length &&
				nextActiveSession === current.active_session
			) {
				return current;
			}

			if (isTemplateWorkspace(current)) {
				return {
					...current,
					active_session: nextActiveSession,
					terminals: nextTerminals,
					browsers: nextBrowsers,
					files: nextFiles,
				};
			}

			return {
				...current,
				active_session: nextActiveSession,
				terminals: nextTerminals,
				browsers: nextBrowsers,
				files: nextFiles,
				unread: nextUnread,
				working: nextWorking,
			};
		},
		[],
	);

	useEffect(() => {
		if (!workspaceName) {
			return;
		}

		let disposed = false;
		const unlisteners: Array<() => void | Promise<void>> = [];

		const disposeListeners = () => {
			for (const unlisten of unlisteners) {
				void Promise.resolve(unlisten()).catch(() => {});
			}
		};

		void listen<{ workspace: string }>("browser://state", (event) => {
			if (disposed || event.payload.workspace !== workspaceName) {
				return;
			}
			invalidateWorkspace();
		}).then((nextUnlisten) => {
			if (disposed) {
				void Promise.resolve(nextUnlisten()).catch(() => {});
				return;
			}
			unlisteners.push(nextUnlisten);
		});

		void listen<WorkspaceStateEventPayload>(
			"workspace://state",
			(event) => {
				if (disposed || event.payload.workspace !== workspaceName) {
					return;
				}
				queryClient.setQueryData<Workspace | null>(
					["workspaces_get_workspace", workspaceName],
					(current) => applyWorkspaceStateEvent(current, event.payload) ?? null,
				);
				invalidateWorkspace();
			},
		).then((nextUnlisten) => {
			if (disposed) {
				void Promise.resolve(nextUnlisten()).catch(() => {});
				return;
			}
			unlisteners.push(nextUnlisten);
		});

		return () => {
			disposed = true;
			disposeListeners();
		};
	}, [applyWorkspaceStateEvent, invalidateWorkspace, queryClient, workspaceName]);

	const workspace = workspaceQuery.data ?? null;
	const isMissing =
		workspaceQuery.isError || (workspaceQuery.isSuccess && workspace == null);

	const value = useMemo(
		() => ({
			workspaceName,
			routeProject: projectParam,
			workspace,
			isLoading: workspaceQuery.isLoading,
			isMissing,
			invalidateWorkspace,
		}),
		[
			invalidateWorkspace,
			isMissing,
			projectParam,
			workspace,
			workspaceName,
			workspaceQuery.isLoading,
		],
	);

	return (
		<WorkspaceStateContext.Provider value={value}>
			{children}
		</WorkspaceStateContext.Provider>
	);
}

export function WorkspaceStateProvider({
	children,
	project,
	workspaceName,
}: {
	children: ReactNode;
	project: string;
	workspaceName: string;
}) {
	return (
		<WorkspaceStateProviderInner
			projectParam={project}
			workspaceName={workspaceName}
		>
			{children}
		</WorkspaceStateProviderInner>
	);
}

export const RouteWorkspaceStateProvider = WorkspaceStateProvider;

function useWorkspaceStateContext() {
	return useContext(WorkspaceStateContext);
}

export function useWorkspaceState() {
	const {
		workspaceName,
		workspace,
		isLoading,
		isMissing,
		invalidateWorkspace,
	} = useWorkspaceStateContext();

	return {
		workspaceName,
		workspace,
		isLoading,
		isMissing,
		invalidateWorkspace,
	};
}

export function useWorkspaceProject() {
	const { routeProject, workspace } = useWorkspaceStateContext();
	const { project } = useWorkspaceRouteParams();

	return workspace?.project ?? routeProject ?? project ?? "";
}

export function useWorkspaceReady() {
	const { workspace } = useWorkspaceStateContext();

	return workspace ? workspaceIsReady(workspace) : false;
}

export function useWorkspaceSessions() {
	const { workspace } = useWorkspaceStateContext();

	return useMemo<WorkspaceSession[]>(
		() =>
			workspace && !isTemplateWorkspace(workspace)
				? workspaceSessions(workspace)
				: [],
		[workspace],
	);
}

export function useCloudSessions() {
	const { workspaceName } = useWorkspaceStateContext();
	const sessions = useWorkspaceSessions();

	return useMemo<CloudSession[]>(
		() =>
			sessions.map((session) =>
				normalizeWorkspaceSession(workspaceName, session),
			),
		[sessions, workspaceName],
	);
}
