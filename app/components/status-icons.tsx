import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { GCloudIcon } from "../icons/gcloud";
import { GHIcon } from "../icons/gh";
import { CodexIcon } from "../icons/codex";
import { ClaudeIcon } from "../icons/claude";
import { Tooltip, TooltipTrigger, TooltipContent } from "./tooltip";
import { toast } from "./toaster";
import { invokeLogged } from "../../lib/logging";

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
	return Promise.race([
		promise,
		new Promise<never>((_, reject) =>
			setTimeout(() => reject(new Error("Timeout")), ms)
		),
	]);
}

function GCloudStatus() {
	const queryClient = useQueryClient();
	const installed = useQuery({
		queryKey: ["gcloud_installed"],
		queryFn: () => invokeLogged<boolean>("gcloud_installed"),
		refetchInterval: 5000,
	});
	const configured = useQuery({
		queryKey: ["gcloud_configured"],
		queryFn: () => invokeLogged<boolean>("gcloud_configured"),
		enabled: installed.data === true,
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () =>
			withTimeout(invokeLogged("gcloud_authenticate"), 10000),
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

	const icon = <GCloudIcon height={12} color="#4285F4" />;

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
					<span style={{ opacity: active ? 1 : 0.5 }} className="flex items-center">
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
		queryKey: ["gh_installed"],
		queryFn: () => invokeLogged<boolean>("gh_installed"),
		refetchInterval: 5000,
	});
	const configured = useQuery({
		queryKey: ["gh_configured"],
		queryFn: () => invokeLogged<boolean>("gh_configured"),
		enabled: installed.data === true,
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invokeLogged("gh_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["gh_configured"] });
			toast({ variant: "success", title: "GitHub CLI authenticated" });
		},
	});

	const active = installed.data && configured.data;
	const canAuth = installed.data && !configured.data;
	const label = !installed.data
		? "GitHub CLI: not installed"
		: !configured.data
			? "GitHub CLI: click to authenticate"
			: "GitHub CLI: connected";

	const icon = <GHIcon height={12} color="#FFFFFF" />;

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
					<span style={{ opacity: active ? 1 : 0.5 }} className="flex items-center">
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
		queryFn: () => invokeLogged<boolean>("codex_configured"),
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () =>
			withTimeout(invokeLogged("codex_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["codex_configured"] });
			toast({ variant: "success", title: "Codex authenticated" });
		},
	});

	const label = configured.data
		? "Codex: connected"
		: "Codex: click to authenticate";

	const icon = <CodexIcon height={12} color="#FFFFFF" />;

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
					<span className="flex items-center">
						{icon}
					</span>
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
		queryFn: () => invokeLogged<boolean>("claude_configured"),
		refetchInterval: 5000,
	});
	const authenticate = useMutation({
		mutationFn: () =>
			withTimeout(invokeLogged("claude_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["claude_configured"] });
			toast({ variant: "success", title: "Claude authenticated" });
		},
	});

	const label = configured.data
		? "Claude: connected"
		: "Claude: click to authenticate";

	const icon = <ClaudeIcon height={12} color="#D97757" />;

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
					<span className="flex items-center">
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="bottom">{label}</TooltipContent>
		</Tooltip>
	);
}

export function StatusIcons() {
	return (
		<div className="flex items-center gap-2">
			<GCloudStatus />
			<GHStatus />
			<CodexStatus />
			<ClaudeStatus />
		</div>
	);
}
