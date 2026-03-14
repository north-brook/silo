import { Box, GitBranch } from "lucide-react";
import { Loader } from "./loader";

interface WorkspaceStatus {
	status: string;
	working?: boolean | null;
	unread?: boolean;
	isTemplate?: boolean;
	optimisticStarting?: boolean;
	optimisticStopping?: boolean;
}

export function WorkspaceIndicator({
	workspace,
}: {
	workspace: WorkspaceStatus;
}) {
	const isStarting =
		workspace.optimisticStarting ||
		workspace.status === "STAGING" ||
		workspace.status === "PROVISIONING";
	const isStopping =
		workspace.optimisticStopping ||
		workspace.status === "STOPPING" ||
		workspace.status === "SUSPENDING";
	const isRunning = workspace.status === "RUNNING";

	if (isStopping) return <Loader className="text-error" />;
	if (isStarting) return <Loader className="text-text-muted" />;
	if (isRunning && workspace.working)
		return <Loader className="text-emerald-400" />;

	const Icon = workspace.isTemplate ? Box : GitBranch;
	return <Icon size={12} className="shrink-0" />;
}

export function workspaceStatusLabel(workspace: WorkspaceStatus): string {
	const isStarting =
		workspace.status === "STAGING" || workspace.status === "PROVISIONING";
	const isStopping =
		workspace.status === "STOPPING" || workspace.status === "SUSPENDING";
	const isRunning = workspace.status === "RUNNING";
	const isStopped =
		workspace.status === "TERMINATED" || workspace.status === "STOPPED";

	if (isStarting) return "Starting...";
	if (isStopping) return "Stopping...";
	if (isRunning && workspace.working) return "Working";
	if (isRunning) return "Running";
	if (isStopped) return "Stopped";
	return workspace.status.charAt(0) + workspace.status.slice(1).toLowerCase();
}
