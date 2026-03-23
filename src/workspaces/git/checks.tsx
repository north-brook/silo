import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Ban,
	Check,
	Clock,
	Ellipsis,
	Minus,
	OctagonAlert,
	RotateCw,
	SkipForward,
	X,
} from "lucide-react";
import Image from "@/shared/ui/image";
import { Loader } from "@/shared/ui/loader";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type {
	CheckState,
	PullRequestChecksSummary,
} from "@/workspaces/git/api";
import {
	gitPrDeployments,
	gitPrDetails,
	gitRerunFailedChecks,
} from "@/workspaces/git/api";
import { useGitSidebar } from "@/workspaces/git/context";

const ACTIVE_PULL_REQUEST_REFRESH_INTERVAL_MS = 15_000;
const TERMINAL_PULL_REQUEST_REFRESH_INTERVAL_MS = 120_000;
const pendingCheckStates: CheckState[] = [
	"in_progress",
	"pending",
	"queued",
	"waiting",
	"requested",
];
const failStates: CheckState[] = ["failure", "startup_failure", "timed_out"];

export function GitChecksStatusIndicator({
	checks,
	isLoading,
}: {
	checks: PullRequestChecksSummary | null;
	isLoading: boolean;
}) {
	if (isLoading) {
		return <span className="w-1.5 h-1.5 rounded-full bg-yellow-400" />;
	}

	if (!checks || checks.total === 0) return null;

	if (checks.has_failing || checks.has_cancelled) {
		return <X size={10} className="text-red-400" />;
	}
	if (checks.has_pending) {
		return <span className="w-1.5 h-1.5 rounded-full bg-yellow-400" />;
	}

	return <Check size={10} className="text-emerald-400" />;
}

export function GitChecksTab() {
	const { workspace, prSummary } = useGitSidebar();
	const queryClient = useQueryClient();
	const detailsQuery = useQuery({
		queryKey: ["git_pr_details", workspace, prSummary?.head_ref_oid ?? null],
		queryFn: () => gitPrDetails(workspace),
		enabled: !!workspace && prSummary?.status === "open",
		refetchInterval: (query) =>
			checksAreTerminal(query.state.data?.checks ?? [])
				? TERMINAL_PULL_REQUEST_REFRESH_INTERVAL_MS
				: ACTIVE_PULL_REQUEST_REFRESH_INTERVAL_MS,
	});
	const deploymentsQuery = useQuery({
		queryKey: [
			"git_pr_deployments",
			workspace,
			prSummary?.head_ref_oid ?? null,
		],
		queryFn: () => gitPrDeployments(workspace),
		enabled: !!workspace && prSummary?.status === "open",
		refetchInterval: (query) =>
			deploymentsAreTerminal(query.state.data ?? [])
				? TERMINAL_PULL_REQUEST_REFRESH_INTERVAL_MS
				: ACTIVE_PULL_REQUEST_REFRESH_INTERVAL_MS,
	});

	const rerunFailed = useMutation({
		mutationFn: () => gitRerunFailedChecks(workspace),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_summary", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_details", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["git_pr_deployments", workspace],
			});
		},
	});

	if (detailsQuery.isLoading) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	if (detailsQuery.isError) {
		return (
			<div className="h-full flex items-center justify-center px-4 text-center text-sm text-text-muted">
				Failed to load checks. {queryErrorMessage(detailsQuery.error)}
			</div>
		);
	}

	if (!detailsQuery.data) {
		return (
			<div className="h-full flex items-center justify-center px-4 text-center text-sm text-text-muted">
				Pull request no longer available
			</div>
		);
	}

	const details = detailsQuery.data;
	const deployments = deploymentsQuery.data ?? [];

	const allResolved =
		details.checks.length > 0 &&
		!details.checks.some((check) => pendingCheckStates.includes(check.state));
	const hasFailed = details.checks.some((check) =>
		failStates.includes(check.state),
	);

	return (
		<div className="flex flex-col gap-3 p-3">
			{details.title && (
				<div>
					<p className="text-sm font-medium text-text-bright">
						{details.title}
					</p>
					{details.body && (
						<p className="text-sm text-text-muted mt-1 line-clamp-3">
							{details.body}
						</p>
					)}
				</div>
			)}

			{(deploymentsQuery.isLoading ||
				deploymentsQuery.isError ||
				deployments.length > 0) && (
				<div className="flex flex-col gap-1">
					<span className="text-sm text-text-muted font-medium uppercase tracking-wide">
						Deployments
					</span>
					{deploymentsQuery.isLoading ? (
						<div className="flex items-center justify-center py-2">
							<Loader />
						</div>
					) : deploymentsQuery.isError ? (
						<p className="py-2 text-sm text-text-muted">
							Failed to load deployments.{" "}
							{queryErrorMessage(deploymentsQuery.error)}
						</p>
					) : (
						deployments.map((deployment) => (
							<button
								key={deployment.id}
								type="button"
								onClick={async () => {
									if (!deployment.url) return;
									const { openUrl } = await import("@tauri-apps/plugin-opener");
									openUrl(deployment.url);
								}}
								className="flex items-center gap-2 px-3 py-1 -mx-3 text-sm hover:bg-btn-hover transition-colors text-left"
							>
								{deployment.icon_url ? (
									<Image
										src={deployment.icon_url}
										alt=""
										unoptimized
										width={14}
										height={14}
										className="w-3.5 h-3.5 rounded shrink-0"
									/>
								) : (
									<StatusDot state={deployment.state} />
								)}
								<span className="text-text truncate flex-1 min-w-0">
									{deployment.url
										? new URL(deployment.url).hostname
										: deployment.environment}
								</span>
							</button>
						))
					)}
				</div>
			)}

			{details.checks.length > 0 && (
				<div className="flex flex-col gap-1">
					<div className="flex items-center justify-between">
						<span className="text-sm text-text-muted font-medium uppercase tracking-wide">
							Checks
						</span>
						{allResolved && hasFailed && (
							<Tooltip>
								<TooltipTrigger asChild>
									<button
										type="button"
										disabled={rerunFailed.isPending}
										onClick={() => rerunFailed.mutate()}
										className="flex items-center text-text-muted hover:text-text transition-colors disabled:opacity-50"
									>
										{rerunFailed.isPending ? <Loader /> : <RotateCw size={9} />}
									</button>
								</TooltipTrigger>
								<TooltipContent side="left">Rerun failed</TooltipContent>
							</Tooltip>
						)}
					</div>
					{[...details.checks]
						.sort((left, right) => left.name.localeCompare(right.name))
						.map((check) => (
							<button
								key={check.id}
								type="button"
								onClick={async () => {
									if (!check.link) return;
									const { openUrl } = await import("@tauri-apps/plugin-opener");
									openUrl(check.link);
								}}
								className="flex items-center gap-2 px-3 py-1 -mx-3 text-sm hover:bg-btn-hover transition-colors text-left"
							>
								<CheckStateIcon state={check.state} />
								<span className="text-text truncate">{check.name}</span>
							</button>
						))}
				</div>
			)}

			{deployments.length === 0 &&
				!deploymentsQuery.isLoading &&
				!deploymentsQuery.isError &&
				details.checks.length === 0 && (
					<p className="text-sm text-text-muted py-4 text-center">
						No checks or deployments
					</p>
				)}
		</div>
	);
}

function StatusDot({ state }: { state: string }) {
	let color: string;
	switch (state) {
		case "success":
			color = "bg-emerald-400";
			break;
		case "failure":
		case "error":
			color = "bg-red-400";
			break;
		case "pending":
		case "in_progress":
		case "queued":
			color = "bg-yellow-400";
			break;
		default:
			color = "bg-text-muted";
			break;
	}

	return <span className={`shrink-0 w-1.5 h-1.5 rounded-full ${color}`} />;
}

function checksAreTerminal(checks: Array<{ state: CheckState }>) {
	return (
		checks.length > 0 &&
		!checks.some((check) => pendingCheckStates.includes(check.state))
	);
}

function deploymentsAreTerminal(deployments: Array<{ state: string }>) {
	return (
		deployments.length > 0 &&
		deployments.every((deployment) =>
			isTerminalDeploymentState(deployment.state),
		)
	);
}

function isTerminalDeploymentState(state: string) {
	const normalized = state
		.trim()
		.toLowerCase()
		.replace(/[\s-]+/g, "_");
	return (
		normalized === "success" ||
		normalized === "failure" ||
		normalized === "error" ||
		normalized === "inactive"
	);
}

export function CheckStateIcon({ state }: { state: CheckState }) {
	const size = 12;

	switch (state) {
		case "success":
			return <Check size={size} className="shrink-0 text-emerald-400" />;
		case "failure":
		case "startup_failure":
			return <X size={size} className="shrink-0 text-red-400" />;
		case "timed_out":
			return <Clock size={size} className="shrink-0 text-red-400" />;
		case "cancelled":
			return <Ban size={size} className="shrink-0 text-text-muted" />;
		case "skipped":
			return <SkipForward size={size} className="shrink-0 text-text-muted" />;
		case "action_required":
			return <OctagonAlert size={size} className="shrink-0 text-yellow-400" />;
		case "in_progress":
			return <Loader className="shrink-0" />;
		case "queued":
		case "waiting":
		case "requested":
		case "pending":
			return <Ellipsis size={size} className="shrink-0 text-yellow-400" />;
		case "neutral":
			return <Minus size={size} className="shrink-0 text-text-muted" />;
		default:
			return <Minus size={size} className="shrink-0 text-text-muted" />;
	}
}

function queryErrorMessage(error: unknown): string {
	return error instanceof Error && error.message ? error.message : "";
}
