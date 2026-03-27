import {
	Box,
	GitBranch,
	GitMerge,
	GitPullRequest,
	GitPullRequestClosed,
} from "lucide-react";
import { Loader } from "@/shared/ui/loader";
import type { WorkspaceLifecycle } from "@/workspaces/api";
import type {
	PullRequestChecksSummary,
	PullRequestSummary,
} from "@/workspaces/git/api";

interface WorkspaceStatus {
	status: string;
	lifecycle: WorkspaceLifecycle;
	working?: boolean | null;
	unread?: boolean;
	isTemplate?: boolean;
	optimisticStarting?: boolean;
	optimisticStopping?: boolean;
	optimisticSuspending?: boolean;
	prSummary?: PullRequestSummary | null;
	dirty?: boolean;
}

function checksColor(
	checks: PullRequestChecksSummary | null | undefined,
): string | null {
	if (!checks || checks.total === 0) return null;
	if (checks.has_failing || checks.has_cancelled) return "text-red-400";
	if (checks.has_pending) return "text-yellow-400";
	return "text-emerald-400";
}

export function WorkspaceIndicator({
	workspace,
}: {
	workspace: WorkspaceStatus;
}) {
	const isSuspending =
		workspace.optimisticSuspending || workspace.status === "SUSPENDING";
	const isSuspended = workspace.status === "SUSPENDED";
	const isStarting =
		workspace.optimisticStarting ||
		workspace.status === "STAGING" ||
		workspace.status === "PROVISIONING";
	const isStopping =
		workspace.optimisticStopping || workspace.status === "STOPPING";
	const isRunning = workspace.status === "RUNNING";
	const isCreating = isRunning && workspace.lifecycle.phase !== "ready";

	if (isSuspending) return <Loader className="text-yellow-400" />;
	if (isStopping) return <Loader className="text-error" />;
	if (isStarting || isCreating) return <Loader className="text-text-muted" />;

	// Template workspaces don't have PR state
	if (workspace.isTemplate) {
		if (isSuspended)
			return <Box size={12} className="shrink-0 text-yellow-400" />;
		return <Box size={12} className="shrink-0" />;
	}

	// Suspended: show yellow tint, keep PR-aware icon shape
	if (isSuspended) {
		const Icon =
			workspace.prSummary?.status === "merged"
				? GitMerge
				: workspace.prSummary?.mergeability === "conflicting"
					? GitPullRequestClosed
					: workspace.prSummary?.status === "open"
						? GitPullRequest
						: GitBranch;
		return <Icon size={12} className="shrink-0 text-yellow-400" />;
	}

	// Determine icon shape from PR lifecycle
	if (workspace.prSummary?.status === "merged")
		return <GitMerge size={12} className="shrink-0 text-text-muted" />;

	if (workspace.prSummary?.status === "open") {
		// Priority: failing > dirty > pending > passing > default
		if (workspace.prSummary.mergeability === "conflicting") {
			return (
				<GitPullRequestClosed size={12} className="shrink-0 text-red-400" />
			);
		}

		let color = "text-text-muted";
		const checkColor = checksColor(workspace.prSummary.checks);
		if (checkColor === "text-red-400") color = "text-red-400";
		else if (workspace.dirty) color = "text-blue-400";
		else if (checkColor) color = checkColor;

		return <GitPullRequest size={12} className={`shrink-0 ${color}`} />;
	}

	// WIP branch (no PR)
	const color = workspace.dirty ? "text-blue-400" : "";
	return <GitBranch size={12} className={`shrink-0 ${color}`} />;
}

export function workspaceStatusLabel(workspace: WorkspaceStatus): string {
	const isSuspending = workspace.status === "SUSPENDING";
	const isSuspended = workspace.status === "SUSPENDED";
	const isStarting =
		workspace.status === "STAGING" || workspace.status === "PROVISIONING";
	const isStopping = workspace.status === "STOPPING";
	const isRunning = workspace.status === "RUNNING";
	const isStopped =
		workspace.status === "TERMINATED" || workspace.status === "STOPPED";
	const isCreating = isRunning && workspace.lifecycle.phase !== "ready";

	if (isSuspending) return "Suspending...";
	if (isSuspended) return "Suspended";
	if (isRunning && workspace.lifecycle.phase === "updating_workspace_agent") {
		return "Updating observer...";
	}
	if (isStarting || isCreating) return "Creating...";
	if (isStopping) return "Stopping...";
	if (isRunning && workspace.lifecycle.phase !== "ready") {
		switch (workspace.lifecycle.phase) {
			case "waiting_for_ssh":
				return "Waiting for SSH...";
			case "bootstrapping":
				return "Preparing...";
			case "waiting_for_agent":
				return "Starting services...";
			case "failed":
				return "Startup failed";
			default:
				return "Starting...";
		}
	}
	if (isRunning && workspace.working) return "Working";
	if (isRunning) return "Running";
	if (isStopped) return "Stopped";
	return workspace.status.charAt(0) + workspace.status.slice(1).toLowerCase();
}
