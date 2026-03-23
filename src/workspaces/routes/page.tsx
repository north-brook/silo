import { useEffect, useMemo, useRef } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Loader } from "@/shared/ui/loader";
import {
	isTemplateWorkspace,
	workspaceIsReady,
	workspaceSessions,
} from "@/workspaces/api";
import { PromptWorkspace } from "@/workspaces/prompt/screen";
import {
	type WorkspaceRouteState,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";
import {
	TemplateOperationScreen,
	WorkspaceResumingScreen,
} from "@/workspaces/routes/transition-screens";
import {
	useTemplateState,
	useWorkspaceProject,
	useWorkspaceState,
} from "@/workspaces/state";
import { TemplatingWorkspace } from "@/workspaces/template/screen";

export default function WorkspacePage() {
	return <WorkspaceView />;
}

function WorkspaceView() {
	const location = useLocation();
	const navigate = useNavigate();
	const savingRoutedRef = useRef(false);
	const routeState = location.state as WorkspaceRouteState | null;
	const freshRef = useRef(routeState?.fresh === true);
	const { isLoading, isMissing, workspace } = useWorkspaceState();
	const project = useWorkspaceProject();
	const transition = routeState?.transition;
	const templateState = useTemplateState(
		(workspace ? isTemplateWorkspace(workspace) : false) || isMissing
			? project
			: null,
	);
	const templateOperation = templateState.data?.operation ?? null;
	const templateLifecycleOperation =
		templateOperation &&
		(templateOperation.kind === "save" || templateOperation.kind === "delete")
			? templateOperation
			: null;

	const redirectHref = useMemo(() => {
		if (!workspace || transition) {
			return null;
		}

		const sessions = workspaceSessions(workspace);
		const activeSession = workspace.active_session
			? sessions.find(
					(session) =>
						session.type === workspace.active_session?.type &&
						session.attachment_id === workspace.active_session?.attachment_id,
				)
			: null;
		const targetSession =
			activeSession ??
			(sessions.length > 0 ? sessions[sessions.length - 1] : null);
		if (!targetSession) {
			return null;
		}

		return workspaceSessionHref({
			project,
			workspace: workspace.name,
			kind: targetSession.type,
			attachmentId: targetSession.attachment_id,
		});
	}, [project, transition, workspace]);

	useEffect(() => {
		if (!freshRef.current) {
			return;
		}

		navigate(location.pathname, {
			replace: true,
			state: transition ? ({ transition } satisfies WorkspaceRouteState) : null,
		});
	}, [location.pathname, navigate, transition]);

	useEffect(() => {
		if (!redirectHref) {
			return;
		}

		navigate(redirectHref, { replace: true });
	}, [navigate, redirectHref]);

	useEffect(() => {
		if (
			!templateLifecycleOperation ||
			templateLifecycleOperation.status !== "completed" ||
			(!isMissing && templateState.data?.workspace_present !== false) ||
			savingRoutedRef.current
		) {
			return;
		}

		savingRoutedRef.current = true;
		const timer = window.setTimeout(
			() => navigate("/", { replace: true }),
			1500,
		);
		return () => {
			window.clearTimeout(timer);
		};
	}, [
		isMissing,
		navigate,
		templateLifecycleOperation,
		templateState.data?.workspace_present,
	]);

	if (templateLifecycleOperation) {
		return <TemplateOperationScreen operation={templateLifecycleOperation} />;
	}

	if (transition === "resuming" && workspace) {
		if (!workspaceIsReady(workspace)) {
			return (
				<WorkspaceResumingScreen
					status={workspace.status}
					lifecycle={workspace.lifecycle}
				/>
			);
		}
	}

	if (isLoading || (!workspace && !isMissing)) {
		return (
			<div className="flex-1 flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (!workspace || isMissing) {
		return null;
	}

	if (redirectHref) {
		return null;
	}

	const isRunning = workspace.status === "RUNNING";

	if (isTemplateWorkspace(workspace)) {
		return (
			<TemplatingWorkspace
				lifecycle={workspace.lifecycle}
				status={workspace.status}
			/>
		);
	}

	return (
		<PromptWorkspace
			autoFocusPrompt={freshRef.current}
			isRunning={isRunning}
			lifecycle={workspace.lifecycle}
			status={workspace.status}
			workspace={workspace.name}
			project={workspace.project}
		/>
	);
}
