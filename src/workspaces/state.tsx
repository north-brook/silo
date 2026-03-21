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
import { useNavigate } from "react-router-dom";
import {
	type BrowserStateEventPayload,
	popupBrowserSessionHrefForEvent,
} from "@/workspaces/browser/events";
import {
	type CloudSession,
	normalizeWorkspaceSession,
} from "@/workspaces/hosts/model";
import { type TemplateState, getTemplateState } from "@/projects/api";
import { useWorkspaceRouteParams } from "@/workspaces/routes/params";
import { invoke } from "@/shared/lib/invoke";
import {
	applyWorkspaceStateEventToWorkspace,
	type WorkspaceStateEventPayload,
} from "@/workspaces/state-events";
import {
	type Workspace,
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

function WorkspaceStateProviderInner({
	children,
	projectParam,
	workspaceName,
}: {
	children: ReactNode;
	projectParam: string;
	workspaceName: string;
}) {
	const navigate = useNavigate();
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

		void listen<BrowserStateEventPayload>("browser://state", (event) => {
			if (disposed || event.payload.workspace !== workspaceName) {
				return;
			}

			const popupHref = popupBrowserSessionHrefForEvent(event.payload, {
				project: projectParam,
				workspaceName,
			});
			const popupAttachmentId = event.payload.popupAttachmentId?.trim();
			if (!popupHref || !popupAttachmentId) {
				invalidateWorkspace();
				return;
			}

			void queryClient
				.invalidateQueries({
					queryKey: ["workspaces_get_workspace", workspaceName],
				})
				.then(() => {
					if (disposed) {
						return;
					}

					const workspace = queryClient.getQueryData<Workspace | null>([
						"workspaces_get_workspace",
						workspaceName,
					]);
					const popupPresent =
						workspace != null &&
						workspaceSessions(workspace).some(
							(session) =>
								session.type === "browser" &&
								session.attachment_id === popupAttachmentId,
						);
					if (popupPresent) {
						navigate(popupHref);
					}
				});
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
					(current) =>
						applyWorkspaceStateEventToWorkspace(current, event.payload) ?? null,
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
	}, [
		invalidateWorkspace,
		navigate,
		projectParam,
		queryClient,
		workspaceName,
	]);

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

export function useTemplateState(project: string | null | undefined) {
	return useQuery<TemplateState>({
		queryKey: ["templates_get_state", project],
		queryFn: () => getTemplateState(project ?? ""),
		enabled: !!project,
		refetchInterval: 2000,
	});
}

export function useWorkspaceReady() {
	const { workspace } = useWorkspaceStateContext();

	return workspace ? workspaceIsReady(workspace) : false;
}

export function useWorkspaceSessions() {
	const { workspace } = useWorkspaceStateContext();

	return useMemo<WorkspaceSession[]>(
		() => (workspace ? workspaceSessions(workspace) : []),
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
