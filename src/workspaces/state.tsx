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
	type WorkspaceSession,
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
		let unlisten: (() => void | Promise<void>) | null = null;

		const disposeListener = (
			nextUnlisten: (() => void | Promise<void>) | null,
		) => {
			if (!nextUnlisten) {
				return;
			}
			void Promise.resolve(nextUnlisten()).catch(() => {});
		};

		void listen<{ workspace: string }>("browser://state", (event) => {
			if (disposed || event.payload.workspace !== workspaceName) {
				return;
			}
			invalidateWorkspace();
		}).then((nextUnlisten) => {
			if (disposed) {
				disposeListener(nextUnlisten);
				return;
			}
			unlisten = nextUnlisten;
		});

		return () => {
			disposed = true;
			disposeListener(unlisten);
		};
	}, [invalidateWorkspace, workspaceName]);

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

	return workspace?.status === "RUNNING" && workspace.ready === true;
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
