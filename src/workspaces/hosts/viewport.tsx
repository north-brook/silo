import { useEffect, useRef } from "react";
import { Loader } from "@/shared/ui/loader";
import type { CloudSession } from "@/workspaces/hosts/model";
import { cloudSessionKey } from "@/workspaces/hosts/model";
import { useSessionHosts } from "@/workspaces/hosts/provider";

export function SessionViewport({
	workspace,
	activeSession,
	skipInitialScrollback,
	bgClassName = "bg-surface",
}: {
	workspace: string;
	activeSession: CloudSession | null;
	skipInitialScrollback: boolean;
	bgClassName?: string;
}) {
	const outletRef = useRef<HTMLDivElement>(null);
	const {
		ensureSession,
		getHost,
		registerWorkspaceOutlet,
		retrySession,
		setActiveSession,
	} = useSessionHosts();
	const activeSessionKey = activeSession
		? cloudSessionKey(activeSession)
		: null;
	const activeHost = getHost(activeSessionKey);

	useEffect(() => {
		registerWorkspaceOutlet(workspace, outletRef.current);
		return () => {
			registerWorkspaceOutlet(workspace, null);
		};
	}, [registerWorkspaceOutlet, workspace]);

	useEffect(() => {
		if (activeSession) {
			ensureSession(activeSession, {
				skipInitialScrollback,
			});
		}
		setActiveSession(workspace, activeSessionKey);
		return () => {
			setActiveSession(null, null);
		};
	}, [
		activeSession,
		activeSessionKey,
		ensureSession,
		setActiveSession,
		skipInitialScrollback,
		workspace,
	]);

	return (
		<div className={`flex-1 min-h-0 ${bgClassName} relative`}>
			{activeSession && (!activeHost || activeHost.status !== "ready") && (
				<div
					className={`absolute inset-0 flex items-center justify-center z-10 ${bgClassName}`}
				>
					<div className="flex items-center gap-2 text-sm text-text-muted">
						{activeHost?.status === "error" ? (
							<>
								<span>
									{activeHost.errorMessage ?? "Session failed to attach"}
								</span>
								<button
									className="rounded border border-line bg-surface-elevated px-2 py-1 text-sm text-text hover:bg-surface"
									onClick={() => retrySession(activeSessionKey)}
									type="button"
								>
									Retry
								</button>
							</>
						) : activeHost?.status === "reconnecting" ? (
							<>
								<Loader />
								<span>Reconnecting to session...</span>
							</>
						) : (
							<>
								<Loader />
								<span>Connecting to session...</span>
							</>
						)}
					</div>
				</div>
			)}
			<div ref={outletRef} className="h-full w-full" />
		</div>
	);
}
