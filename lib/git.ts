import { invoke } from "./invoke";

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
	log_excerpt: string;
	log_truncated: boolean;
	log_available: boolean;
}

export interface PullRequestObservation {
	title: string | null;
	body: string | null;
	deployments: Deployment[];
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

export function gitPrObserve(
	workspace: string,
): Promise<PullRequestObservation | null> {
	return invoke<PullRequestObservation | null>(
		"git_pr_observe",
		{ workspace },
		{
			log: "state_changes_only",
			key: `poll:git_pr_observe:${workspace}`,
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

export function gitPush(workspace: string): Promise<GitTerminalResult> {
	return invoke<GitTerminalResult>("git_push", { workspace });
}

export function gitCreatePr(workspace: string): Promise<GitTerminalResult> {
	return invoke<GitTerminalResult>("git_create_pr", { workspace });
}

export function gitMergePr(workspace: string): Promise<void> {
	return invoke<void>("git_merge_pr", { workspace });
}
