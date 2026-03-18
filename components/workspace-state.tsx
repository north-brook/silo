"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { usePathname, useSearchParams } from "next/navigation";
import {
	createContext,
	type ReactNode,
	Suspense,
	useCallback,
	useContext,
	useEffect,
	useMemo,
} from "react";
import {
	type CloudSession,
	normalizeWorkspaceSession,
} from "../lib/cloud";
import { invoke } from "../lib/invoke";
import {
	isTemplateWorkspace,
	type Workspace,
	type WorkspaceSession,
	workspaceSessions,
} from "../lib/workspaces";

interface WorkspaceStateContextValue {
	workspaceName: string;
	project: string;
	workspace: Workspace | null;
	isLoading: boolean;
	isWorkspaceReady: boolean;
	sessions: WorkspaceSession[];
	cloudSessions: CloudSession[];
	invalidateWorkspace: () => void;
}

const WorkspaceStateContext = createContext<WorkspaceStateContextValue>({
	workspaceName: "",
	project: "",
	workspace: null,
	isLoading: false,
	isWorkspaceReady: false,
	sessions: [],
	cloudSessions: [],
	invalidateWorkspace: () => {},
});

function WorkspaceStateProviderInner({
	children,
}: {
	children: ReactNode;
}) {
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const queryClient = useQueryClient();
	const isWorkspaceShellRoute =
		pathname === "/workspace" || pathname === "/workspace/session";
	const workspaceName = isWorkspaceShellRoute
		? (searchParams.get("name") ?? searchParams.get("workspace") ?? "")
		: "";
	const projectParam = isWorkspaceShellRoute
		? (searchParams.get("project") ?? "")
		: "";

	const workspaceQuery = useQuery({
		queryKey: ["workspaces_get_workspace", workspaceName],
		queryFn: () =>
			invoke<Workspace>(
				"workspaces_get_workspace",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:workspaces_get_workspace:${workspaceName}`,
				},
			),
		enabled: isWorkspaceShellRoute && !!workspaceName,
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
		if (!isWorkspaceShellRoute || !workspaceName) {
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
	}, [invalidateWorkspace, isWorkspaceShellRoute, workspaceName]);

	const workspace = workspaceQuery.data ?? null;
	const project = projectParam || workspace?.project || "";
	const sessions = useMemo<WorkspaceSession[]>(
		() =>
			workspace && !isTemplateWorkspace(workspace)
				? workspaceSessions(workspace)
				: [],
		[workspace],
	);
	const cloudSessions = useMemo(
		() =>
			sessions.map((session) =>
				normalizeWorkspaceSession(workspaceName, session),
			),
		[sessions, workspaceName],
	);

	const value = useMemo(
		() => ({
			workspaceName,
			project,
			workspace,
			isLoading: workspaceQuery.isLoading,
			isWorkspaceReady:
				workspace?.status === "RUNNING" && workspace.ready === true,
			sessions,
			cloudSessions,
			invalidateWorkspace,
		}),
		[
			cloudSessions,
			invalidateWorkspace,
			project,
			sessions,
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

export function WorkspaceStateProvider({ children }: { children: ReactNode }) {
	return (
		<Suspense fallback={children}>
			<WorkspaceStateProviderInner>{children}</WorkspaceStateProviderInner>
		</Suspense>
	);
}

export function useWorkspaceState() {
	return useContext(WorkspaceStateContext);
}
