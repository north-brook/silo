import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "../../lib/invoke";
import { ChromeIcon } from "../icons/chrome";
import { ClaudeIcon } from "../icons/claude";
import { CodexIcon } from "../icons/codex";
import { GCloudIcon } from "../icons/gcloud";
import { GHIcon } from "../icons/gh";
import { toast } from "./toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
	return Promise.race([
		promise,
		new Promise<never>((_, reject) =>
			setTimeout(() => reject(new Error("Timeout")), ms),
		),
	]);
}

function GCloudStatus() {
	const queryClient = useQueryClient();
	const installed = useQuery({
		queryKey: ["gcloud_installed"],
		queryFn: () =>
			invoke<boolean>("gcloud_installed", {
				log: "state_changes_only",
				key: "poll:gcloud_installed",
			}),
		refetchInterval: 5000,
	});
	const configured = useQuery({
		queryKey: ["gcloud_configured"],
		queryFn: () =>
			invoke<boolean>("gcloud_configured", {
				log: "state_changes_only",
				key: "poll:gcloud_configured",
			}),
		enabled: installed.data === true,
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke("gcloud_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["gcloud_configured"] });
			toast({ variant: "success", title: "Google Cloud authenticated" });
		},
	});

	const active = installed.data && configured.data;
	const canAuth = installed.data && !configured.data;
	const label = !installed.data
		? "Google Cloud: not installed"
		: !configured.data
			? "Google Cloud: click to authenticate"
			: "Google Cloud: connected";

	const icon = <GCloudIcon height={12} />;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				{canAuth ? (
					<button
						type="button"
						onClick={() => authenticate.mutate()}
						className={`flex items-center ${authenticate.isPending ? "animate-pulse" : "opacity-50 hover:opacity-100 transition-opacity duration-150"}`}
					>
						{icon}
					</button>
				) : (
					<span
						style={{ opacity: active ? 1 : 0.5 }}
						className="flex items-center"
					>
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="bottom">{label}</TooltipContent>
		</Tooltip>
	);
}

function GHStatus() {
	const queryClient = useQueryClient();
	const installed = useQuery({
		queryKey: ["git_installed"],
		queryFn: () =>
			invoke<boolean>("git_installed", {
				log: "state_changes_only",
				key: "poll:git_installed",
			}),
		refetchInterval: 5000,
	});
	const configured = useQuery({
		queryKey: ["git_configured"],
		queryFn: () =>
			invoke<boolean>("git_configured", {
				log: "state_changes_only",
				key: "poll:git_configured",
			}),
		enabled: installed.data === true,
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke("git_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["git_configured"] });
			toast({ variant: "success", title: "GitHub authenticated" });
		},
	});

	const active = installed.data && configured.data;
	const canAuth = installed.data && !configured.data;
	const label = !installed.data
		? "GitHub: not installed"
		: !configured.data
			? "GitHub: click to authenticate"
			: "GitHub: connected";

	const icon = <GHIcon height={12} />;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				{canAuth ? (
					<button
						type="button"
						onClick={() => authenticate.mutate()}
						className={`flex items-center ${authenticate.isPending ? "animate-pulse" : "opacity-50 hover:opacity-100 transition-opacity duration-150"}`}
					>
						{icon}
					</button>
				) : (
					<span
						style={{ opacity: active ? 1 : 0.5 }}
						className="flex items-center"
					>
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="bottom">{label}</TooltipContent>
		</Tooltip>
	);
}

function CodexStatus() {
	const queryClient = useQueryClient();
	const configured = useQuery({
		queryKey: ["codex_configured"],
		queryFn: () =>
			invoke<boolean>("codex_configured", {
				log: "state_changes_only",
				key: "poll:codex_configured",
			}),
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke("codex_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["codex_configured"] });
			toast({ variant: "success", title: "Codex authenticated" });
		},
	});

	const label = configured.data
		? "Codex: connected"
		: "Codex: click to authenticate";

	const icon = <CodexIcon height={12} />;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				{!configured.data ? (
					<button
						type="button"
						onClick={() => authenticate.mutate()}
						className={`flex items-center ${authenticate.isPending ? "animate-pulse" : "opacity-50 hover:opacity-100 transition-opacity duration-150"}`}
					>
						{icon}
					</button>
				) : (
					<span className="flex items-center">{icon}</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="bottom">{label}</TooltipContent>
		</Tooltip>
	);
}

function ClaudeStatus() {
	const queryClient = useQueryClient();
	const configured = useQuery({
		queryKey: ["claude_configured"],
		queryFn: () =>
			invoke<boolean>("claude_configured", {
				log: "state_changes_only",
				key: "poll:claude_configured",
			}),
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke("claude_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["claude_configured"] });
			toast({ variant: "success", title: "Claude authenticated" });
		},
	});

	const label = configured.data
		? "Claude: connected"
		: "Claude: click to authenticate";

	const icon = <ClaudeIcon height={12} />;

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				{!configured.data ? (
					<button
						type="button"
						onClick={() => authenticate.mutate()}
						className={`flex items-center ${authenticate.isPending ? "animate-pulse" : "opacity-50 hover:opacity-100 transition-opacity duration-150"}`}
					>
						{icon}
					</button>
				) : (
					<span className="flex items-center">{icon}</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="bottom">{label}</TooltipContent>
		</Tooltip>
	);
}

function ChromeStatus() {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<span className="flex items-center opacity-50">
					<ChromeIcon height={12} />
				</span>
			</TooltipTrigger>
			<TooltipContent side="bottom">Chrome: not configured</TooltipContent>
		</Tooltip>
	);
}

export function StatusIcons() {
	return (
		<div className="flex items-center gap-2">
			<GCloudStatus />
			<GHStatus />
			<ChromeStatus />
			<CodexStatus />
			<ClaudeStatus />
		</div>
	);
}
