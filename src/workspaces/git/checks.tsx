import { useMutation, useQueryClient } from "@tanstack/react-query";
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
import type { CheckState, PullRequestObservation } from "@/workspaces/git/api";
import { gitRerunFailedChecks } from "@/workspaces/git/api";
import { useGitSidebar } from "@/workspaces/git/context";
import Image from "@/shared/ui/image";
import { Loader } from "@/shared/ui/loader";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function GitChecksStatusIndicator({
	observation,
	isLoading,
}: {
	observation: PullRequestObservation | null;
	isLoading: boolean;
}) {
	if (isLoading) {
		return <span className="w-1.5 h-1.5 rounded-full bg-yellow-400" />;
	}

	const checks = observation?.checks ?? [];
	if (checks.length === 0) return null;

	const failStates: CheckState[] = [
		"failure",
		"startup_failure",
		"timed_out",
		"cancelled",
	];
	const pendingStates: CheckState[] = [
		"in_progress",
		"pending",
		"queued",
		"waiting",
		"requested",
	];

	if (checks.some((check) => failStates.includes(check.state))) {
		return <X size={10} className="text-red-400" />;
	}
	if (checks.some((check) => pendingStates.includes(check.state))) {
		return <span className="w-1.5 h-1.5 rounded-full bg-yellow-400" />;
	}

	return <Check size={10} className="text-emerald-400" />;
}

export function GitChecksTab({
	observation,
	isLoading,
}: {
	observation: PullRequestObservation | null;
	isLoading: boolean;
}) {
	const { workspace } = useGitSidebar();
	const queryClient = useQueryClient();

	const rerunFailed = useMutation({
		mutationFn: () => gitRerunFailedChecks(workspace),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["git_pr_observe", workspace],
			});
		},
	});

	if (isLoading || !observation) {
		return (
			<div className="h-full flex items-center justify-center">
				<Loader />
			</div>
		);
	}

	const pendingStates: CheckState[] = [
		"in_progress",
		"pending",
		"queued",
		"waiting",
		"requested",
	];
	const failStates: CheckState[] = ["failure", "startup_failure", "timed_out"];
	const allResolved =
		observation.checks.length > 0 &&
		!observation.checks.some((check) => pendingStates.includes(check.state));
	const hasFailed = observation.checks.some((check) =>
		failStates.includes(check.state),
	);

	return (
		<div className="flex flex-col gap-3 p-3">
			{observation.title && (
				<div>
					<p className="text-[11px] font-medium text-text-bright">
						{observation.title}
					</p>
					{observation.body && (
						<p className="text-[11px] text-text-muted mt-1 line-clamp-3">
							{observation.body}
						</p>
					)}
				</div>
			)}

			{observation.deployments.length > 0 && (
				<div className="flex flex-col gap-1">
					<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
						Deployments
					</span>
					{observation.deployments.map((deployment) => (
						<button
							key={deployment.id}
							type="button"
							onClick={async () => {
								if (!deployment.url) return;
								const { openUrl } = await import("@tauri-apps/plugin-opener");
								openUrl(deployment.url);
							}}
							className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
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
					))}
				</div>
			)}

			{observation.checks.length > 0 && (
				<div className="flex flex-col gap-1">
					<div className="flex items-center justify-between">
						<span className="text-[10px] text-text-muted font-medium uppercase tracking-wide">
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
					{[...observation.checks]
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
								className="flex items-center gap-2 px-3 py-1 -mx-3 text-[11px] hover:bg-btn-hover transition-colors text-left"
							>
								<CheckStateIcon state={check.state} />
								<span className="text-text truncate">{check.name}</span>
							</button>
						))}
				</div>
			)}

			{observation.deployments.length === 0 &&
				observation.checks.length === 0 && (
					<p className="text-[11px] text-text-muted py-4 text-center">
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
