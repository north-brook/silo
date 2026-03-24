import { useEffect, useMemo, useRef } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { domFocusSnapshot } from "@/shared/lib/focus-debug";
import { invoke } from "@/shared/lib/invoke";
import { BrowserSessionView } from "@/workspaces/browser/view";
import { WorkspaceFileSessionView } from "@/workspaces/files/view";
import type { CloudSession } from "@/workspaces/hosts/model";
import { useWorkspaceSessionRouteParams } from "@/workspaces/routes/params";
import {
  type SessionRouteState,
  workspaceHref,
  workspaceSessionHref,
} from "@/workspaces/routes/paths";
import {
  useCloudSessions,
  useWorkspaceSessions,
  useWorkspaceState,
} from "@/workspaces/state";
import { WorkspaceUpdatingScreen } from "@/workspaces/routes/updating";
import { TerminalSessionView } from "@/workspaces/terminal/view";

export function WorkspaceBrowserSessionPage() {
  return <WorkspaceSessionView kind="browser" />;
}

export function WorkspaceTerminalSessionPage() {
  return <WorkspaceSessionView kind="terminal" />;
}

export function WorkspaceFileSessionPage() {
  const { workspace } = useWorkspaceState();

  if (workspace?.lifecycle.phase === "updating_workspace_agent") {
    return <WorkspaceUpdatingScreen lifecycle={workspace.lifecycle} />;
  }

  return <WorkspaceFileSessionView />;
}

function WorkspaceSessionView({ kind }: { kind: "browser" | "terminal" }) {
  const location = useLocation();
  const navigate = useNavigate();
  const routeState = location.state as SessionRouteState | null;
  const { invalidateWorkspace, workspace: currentWorkspace } =
    useWorkspaceState();
  const sessions = useWorkspaceSessions();
  const cloudSessions = useCloudSessions();
  const {
    attachmentId,
    project,
    workspaceName: workspace,
  } = useWorkspaceSessionRouteParams();
  const freshRouteRef = useRef<{
    routeKey: string;
    fresh: boolean;
  } | null>(null);
  const routeKey = `${kind}:${attachmentId ?? ""}`;

  if (freshRouteRef.current?.routeKey !== routeKey) {
    freshRouteRef.current = {
      routeKey,
      fresh: routeState?.fresh === true,
    };
  }

  const isFreshRoute = freshRouteRef.current?.fresh === true;

  useEffect(() => {
    if (!isFreshRoute) {
      return;
    }

    navigate(
      workspaceSessionHref({
        project,
        workspace,
        kind,
        attachmentId,
      }),
      { replace: true, state: null },
    );
  }, [attachmentId, isFreshRoute, kind, navigate, project, workspace]);

  const hasLiveSession = useMemo(
    () =>
      sessions.some(
        (session) =>
          session.type === kind && session.attachment_id === attachmentId,
      ),
    [attachmentId, kind, sessions],
  );

  const activeSession = useMemo<CloudSession | null>(() => {
    if (!workspace || !attachmentId) {
      return null;
    }

    return (
      cloudSessions.find(
        (session) =>
          session.kind === kind && session.attachmentId === attachmentId,
      ) ??
      (isFreshRoute
        ? {
            workspace,
            kind,
            attachmentId,
            name: attachmentId,
            url: null,
            logicalUrl: null,
            resolvedUrl: null,
            title: null,
            faviconUrl: null,
            canGoBack: null,
            canGoForward: null,
            working: null,
            unread: null,
          }
        : null)
    );
  }, [attachmentId, cloudSessions, isFreshRoute, kind, workspace]);

  useEffect(() => {
    if (!workspace || !attachmentId || isFreshRoute || hasLiveSession) {
      return;
    }

    navigate(workspaceHref({ project, workspace }), { replace: true });
  }, [attachmentId, hasLiveSession, isFreshRoute, navigate, project, workspace]);

  useEffect(() => {
    if (!workspace || !attachmentId || !hasLiveSession) {
      return;
    }

    console.info("workspace session route active", {
      workspace,
      kind,
      attachmentId,
      fresh: isFreshRoute,
      ...domFocusSnapshot(),
    });

    const timeout = window.setTimeout(() => {
      console.info("workspace session set active requested", {
        workspace,
        kind,
        attachmentId,
        ...domFocusSnapshot(),
      });
      void invoke("workspaces_set_active_session", {
        workspace,
        kind,
        attachmentId,
      });
    }, 200);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [attachmentId, hasLiveSession, isFreshRoute, kind, workspace]);

  if (currentWorkspace?.lifecycle.phase === "updating_workspace_agent") {
    return <WorkspaceUpdatingScreen lifecycle={currentWorkspace.lifecycle} />;
  }

  if (!workspace || !attachmentId || !activeSession) {
    return null;
  }

  if (kind === "terminal") {
    return (
      <TerminalSessionView
        session={activeSession}
        skipInitialScrollback={isFreshRoute}
      />
    );
  }

  return (
    <BrowserSessionView
      session={activeSession}
      autoFocusAddress={isFreshRoute}
      onChanged={invalidateWorkspace}
    />
  );
}
