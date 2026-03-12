import { TerminalLoader } from "./terminal-loader";

interface WorkspaceStatus {
	status: string;
	working?: boolean | null;
	unread?: boolean;
}

export function WorkspaceIndicator({ workspace }: { workspace: WorkspaceStatus }) {
	const isStarting = workspace.status === "STAGING" || workspace.status === "PROVISIONING";
	const isStopping = workspace.status === "STOPPING" || workspace.status === "SUSPENDING";
	const isRunning = workspace.status === "RUNNING";

	if (isStarting) return <TerminalLoader className="text-text-muted" />;
	if (isStopping) return <TerminalLoader className="text-error" />;
	if (isRunning && workspace.working) return <TerminalLoader className="text-emerald-400" />;
	if (isRunning && workspace.unread) return <span className="w-1.5 h-1.5 shrink-0 bg-accent" />;
	if (isRunning) return <span className="w-1.5 h-1.5 shrink-0 bg-emerald-400" />;
	return <span className="w-1.5 h-1.5 shrink-0 bg-text-muted" />;
}

export function workspaceStatusLabel(workspace: WorkspaceStatus): string {
	const isStarting = workspace.status === "STAGING" || workspace.status === "PROVISIONING";
	const isStopping = workspace.status === "STOPPING" || workspace.status === "SUSPENDING";
	const isRunning = workspace.status === "RUNNING";
	const isStopped = workspace.status === "TERMINATED" || workspace.status === "STOPPED";

	if (isStarting) return "Starting...";
	if (isStopping) return "Stopping...";
	if (isRunning && workspace.working) return "Working";
	if (isRunning) return "Running";
	if (isStopped) return "Stopped";
	return workspace.status.charAt(0) + workspace.status.slice(1).toLowerCase();
}
