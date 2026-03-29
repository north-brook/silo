import { invoke } from "@/shared/lib/invoke";

export interface DiffOverview {
	additions: number;
	deletions: number;
	files_changed: number;
}

export type DiffFileStatus =
	| "added"
	| "modified"
	| "deleted"
	| "renamed"
	| "copied"
	| "type_changed"
	| "unmerged"
	| "unknown";

export interface DiffFile {
	path: string;
	previous_path: string | null;
	status: DiffFileStatus;
	additions: number;
	deletions: number;
	binary: boolean;
	patch: string | null;
}

export interface DiffSection {
	overview: DiffOverview;
	files: DiffFile[];
}

export interface Diff {
	overview: DiffOverview;
	local: DiffSection;
	remote: DiffSection;
}

export interface Deployment {
	id: string;
	environment: string;
	state: string;
	description: string;
	url: string | null;
	created_at: string | null;
	updated_at: string | null;
	icon_url: string | null;
}

export type CheckState =
	| "queued"
	| "in_progress"
	| "pending"
	| "requested"
	| "waiting"
	| "success"
	| "failure"
	| "cancelled"
	| "skipped"
	| "neutral"
	| "action_required"
	| "timed_out"
	| "startup_failure"
	| "stale"
	| "unknown";

export interface Check {
	id: string;
	name: string;
	workflow: string | null;
	state: CheckState;
	bucket: string | null;
	description: string | null;
	link: string | null;
	started_at: string | null;
	completed_at: string | null;
}

export interface PullRequestChecksSummary {
	total: number;
	has_pending: boolean;
	has_failing: boolean;
	has_cancelled: boolean;
}

export type PullRequestMergeability = "mergeable" | "conflicting" | "unknown";

export interface PullRequestSummary {
	status: "open" | "closed" | "merged";
	number: number;
	url: string;
	head_ref_oid: string;
	mergeability?: PullRequestMergeability | null;
	checks: PullRequestChecksSummary | null;
}

export interface PullRequestDetails {
	title: string | null;
	body: string | null;
	checks: Check[];
}

export interface PullRequestStatus {
	status: "open" | "closed" | "merged";
	number: number;
	url: string;
}

export interface GitTerminalResult {
	attachment_id: string;
}

export function gitDiff(workspace: string): Promise<Diff> {
	return invoke<Diff>(
		"git_diff",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_diff:${workspace}`,
		},
	);
}

export function gitDiffSummary(workspace: string): Promise<Diff> {
	return invoke<Diff>(
		"git_diff_summary",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_diff_summary:${workspace}`,
		},
	);
}

export function gitDiffFile(
	workspace: string,
	path: string,
): Promise<DiffFile | null> {
	return invoke<DiffFile | null>(
		"git_diff_file",
		{ workspace, path },
		{
			log: "state_changes_only",
			key: `poll:git_diff_file:${workspace}:${path}`,
		},
	);
}

export function gitPrStatus(
	workspace: string,
): Promise<PullRequestStatus | null> {
	return invoke<PullRequestStatus | null>(
		"git_pr_status",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_pr_status:${workspace}`,
		},
	);
}

export function gitPrSummary(
	workspace: string,
): Promise<PullRequestSummary | null> {
	return invoke<PullRequestSummary | null>(
		"git_pr_summary",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_pr_summary:${workspace}`,
		},
	);
}

export function gitPrDetails(
	workspace: string,
): Promise<PullRequestDetails | null> {
	return invoke<PullRequestDetails | null>(
		"git_pr_details",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_pr_details:${workspace}`,
		},
	);
}

export function gitPrDeployments(workspace: string): Promise<Deployment[]> {
	return invoke<Deployment[]>(
		"git_pr_deployments",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_pr_deployments:${workspace}`,
		},
	);
}

export function gitTreeDirty(workspace: string): Promise<boolean> {
	return invoke<boolean>(
		"git_tree_dirty",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_tree_dirty:${workspace}`,
		},
	);
}

export function gitUpdateBranch(
	workspace: string,
	branch: string,
): Promise<void> {
	return invoke<void>("git_update_branch", { workspace, branch });
}

export function gitUpdateTargetBranch(
	workspace: string,
	targetBranch: string,
): Promise<void> {
	return invoke<void>("git_update_target_branch", {
		workspace,
		target_branch: targetBranch,
	});
}

export function gitPush(workspace: string): Promise<GitTerminalResult> {
	return invoke<GitTerminalResult>("git_push", { workspace });
}

export function gitCreatePr(workspace: string): Promise<GitTerminalResult> {
	return invoke<GitTerminalResult>("git_create_pr", { workspace });
}

export function gitMergePr(workspace: string): Promise<void> {
	return invoke<void>("git_merge_pr", { workspace });
}

export function gitResolveConflicts(
	workspace: string,
): Promise<GitTerminalResult> {
	return invoke<GitTerminalResult>("git_resolve_conflicts", { workspace });
}

export function gitRerunFailedChecks(workspace: string): Promise<void> {
	return invoke<void>("git_rerun_failed_checks", { workspace });
}
