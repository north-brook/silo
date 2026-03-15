import { Box, GitBranch } from "lucide-react";
import { Loader } from "./loader";

interface WorkspaceStatus {
	status: string;
	ready?: boolean;
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
	const isCreating = isRunning && !workspace.ready;

	if (isStopping) return <Loader className="text-error" />;
	if (isStarting || isCreating) return <Loader className="text-text-muted" />;

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
	const isCreating = isRunning && !workspace.ready;

	if (isStarting || isCreating) return "Creating...";
	if (isStopping) return "Stopping...";
	if (isRunning && workspace.working) return "Working";
	if (isRunning) return "Running";
	if (isStopped) return "Stopped";
	return workspace.status.charAt(0) + workspace.status.slice(1).toLowerCase();
}
