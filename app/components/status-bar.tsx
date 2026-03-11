import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { GCloudIcon } from "../icons/gcloud";
import { GHIcon } from "../icons/gh";
import { CodexIcon } from "../icons/codex";
import { ClaudeIcon } from "../icons/claude";
import { Tooltip, TooltipTrigger, TooltipContent } from "./tooltip";
import { toast } from "../hooks/use-toast";

function GCloudStatus() {
	const queryClient = useQueryClient();
	const installed = useQuery({
		queryKey: ["gcloud_installed"],
		queryFn: () => invoke<boolean>("gcloud_installed"),
	});
	const configured = useQuery({
		queryKey: ["gcloud_configured"],
		queryFn: () => invoke<boolean>("gcloud_configured"),
		enabled: installed.data === true,
	});
	const authenticate = useMutation({
		mutationFn: () => invoke("gcloud_authenticate"),
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
						disabled={authenticate.isPending}
						style={{ opacity: 0.5 }}
						className="flex items-center"
					>
						{icon}
					</button>
				) : (
					<span style={{ opacity: active ? 1 : 0.5 }} className="flex items-center">
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="top">{label}</TooltipContent>
		</Tooltip>
	);
}

function GHStatus() {
	const queryClient = useQueryClient();
	const installed = useQuery({
		queryKey: ["gh_installed"],
		queryFn: () => invoke<boolean>("gh_installed"),
	});
	const configured = useQuery({
		queryKey: ["gh_configured"],
		queryFn: () => invoke<boolean>("gh_configured"),
		enabled: installed.data === true,
	});
	const authenticate = useMutation({
		mutationFn: () => invoke("gh_authenticate"),
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
						disabled={authenticate.isPending}
						style={{ opacity: 0.5 }}
						className="flex items-center"
					>
						{icon}
					</button>
				) : (
					<span style={{ opacity: active ? 1 : 0.5 }} className="flex items-center">
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="top">{label}</TooltipContent>
		</Tooltip>
	);
}

function CodexStatus() {
	const queryClient = useQueryClient();
	const configured = useQuery({
		queryKey: ["codex_configured"],
		queryFn: () => invoke<boolean>("codex_configured"),
	});
	const authenticate = useMutation({
		mutationFn: () => invoke("codex_authenticate"),
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
						disabled={authenticate.isPending}
						style={{ opacity: 0.5 }}
						className="flex items-center"
					>
						{icon}
					</button>
				) : (
					<span className="flex items-center">
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="top">{label}</TooltipContent>
		</Tooltip>
	);
}

function ClaudeStatus() {
	const queryClient = useQueryClient();
	const configured = useQuery({
		queryKey: ["claude_configured"],
		queryFn: () => invoke<boolean>("claude_configured"),
	});
	const authenticate = useMutation({
		mutationFn: () => invoke("claude_authenticate"),
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
						disabled={authenticate.isPending}
						style={{ opacity: 0.5 }}
						className="flex items-center"
					>
						{icon}
					</button>
				) : (
					<span className="flex items-center">
						{icon}
					</span>
				)}
			</TooltipTrigger>
			<TooltipContent side="top">{label}</TooltipContent>
		</Tooltip>
	);
}

export function StatusBar() {
	return (
		<footer className="fixed bottom-0 left-0 right-0 h-6 flex items-center justify-between px-3 text-[11px] border-t border-border-light bg-bg">
			<span className="text-text-muted">Silo v0.1.0</span>
			<div className="flex items-center gap-2">
				<GCloudStatus />
				<GHStatus />
				<CodexStatus />
				<ClaudeStatus />
			</div>
		</footer>
	);
}
