import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpRight, Check } from "lucide-react";
import type { ReactNode } from "react";
import HomePage from "@/dashboard/page";
import { invoke } from "@/shared/lib/invoke";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { GHIcon } from "@/shared/ui/icons/gh";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";

const GCLOUD_INSTALL_URL = "https://cloud.google.com/sdk/docs/install";
const GITHUB_INSTALL_URL = "https://cli.github.com/";

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
	return Promise.race([
		promise,
		new Promise<never>((_, reject) =>
			setTimeout(() => reject(new Error("Timeout")), ms),
		),
	]);
}

type ServiceConnection = {
	id: string;
	name: string;
	icon: ReactNode;
	ready: boolean;
	connected: boolean;
	isActing: boolean;
	actionLabel: string | null;
	onAction: (() => void) | null;
};

function mutationErrorMessage(error: unknown, fallback: string) {
	return error instanceof Error && error.message.trim()
		? error.message
		: fallback;
}

function installLink(url: string) {
	return async () => {
		const { openUrl } = await import("@tauri-apps/plugin-opener");
		await openUrl(url);
	};
}

function useGCloudConnection(): ServiceConnection {
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
	const install = useMutation({
		mutationFn: installLink(GCLOUD_INSTALL_URL),
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to open Google Cloud install guide",
				description: mutationErrorMessage(
					error,
					"Could not open install guide",
				),
			});
		},
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke<void>("gcloud_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["gcloud_configured"] });
			toast({ variant: "success", title: "Google Cloud authenticated" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Google Cloud authentication failed",
				description: mutationErrorMessage(
					error,
					"Could not authenticate Google Cloud",
				),
			});
		},
	});

	const ready =
		installed.data !== undefined &&
		(installed.data === false || configured.data !== undefined);
	const connected = installed.data === true && configured.data === true;
	const actionLabel = connected
		? null
		: installed.data === false
			? "Install"
			: installed.data === true && configured.data === false
				? "Connect"
				: null;
	const onAction =
		installed.data === false
			? () => install.mutate()
			: installed.data === true && configured.data === false
				? () => authenticate.mutate()
				: null;

	return {
		id: "gcloud",
		name: "Google Cloud",
		icon: <GCloudIcon height={14} />,
		ready,
		connected,
		isActing: install.isPending || authenticate.isPending,
		actionLabel,
		onAction,
	};
}

function useGitHubConnection(): ServiceConnection {
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
	const install = useMutation({
		mutationFn: installLink(GITHUB_INSTALL_URL),
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to open GitHub CLI install guide",
				description: mutationErrorMessage(
					error,
					"Could not open install guide",
				),
			});
		},
	});
	const authenticate = useMutation({
		mutationFn: () => withTimeout(invoke<void>("git_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["git_configured"] });
			toast({ variant: "success", title: "GitHub authenticated" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "GitHub authentication failed",
				description: mutationErrorMessage(
					error,
					"Could not authenticate GitHub",
				),
			});
		},
	});

	const ready =
		installed.data !== undefined &&
		(installed.data === false || configured.data !== undefined);
	const connected = installed.data === true && configured.data === true;
	const actionLabel = connected
		? null
		: installed.data === false
			? "Install"
			: installed.data === true && configured.data === false
				? "Connect"
				: null;
	const onAction =
		installed.data === false
			? () => install.mutate()
			: installed.data === true && configured.data === false
				? () => authenticate.mutate()
				: null;

	return {
		id: "github",
		name: "GitHub",
		icon: <GHIcon height={14} />,
		ready,
		connected,
		isActing: install.isPending || authenticate.isPending,
		actionLabel,
		onAction,
	};
}

function useCodexConnection(): ServiceConnection {
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
		mutationFn: () => withTimeout(invoke<void>("codex_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["codex_configured"] });
			toast({ variant: "success", title: "Codex authenticated" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Codex authentication failed",
				description: mutationErrorMessage(
					error,
					"Could not authenticate Codex",
				),
			});
		},
	});

	return {
		id: "codex",
		name: "Codex",
		icon: <CodexIcon height={14} />,
		ready: configured.data !== undefined,
		connected: configured.data === true,
		isActing: authenticate.isPending,
		actionLabel: configured.data === true ? null : "Connect",
		onAction: configured.data === true ? null : () => authenticate.mutate(),
	};
}

function useClaudeConnection(): ServiceConnection {
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
		mutationFn: () => withTimeout(invoke<void>("claude_authenticate"), 10000),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["claude_configured"] });
			toast({ variant: "success", title: "Claude authenticated" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Claude authentication failed",
				description: mutationErrorMessage(
					error,
					"Could not authenticate Claude",
				),
			});
		},
	});

	return {
		id: "claude",
		name: "Claude",
		icon: <ClaudeIcon height={14} />,
		ready: configured.data !== undefined,
		connected: configured.data === true,
		isActing: authenticate.isPending,
		actionLabel: configured.data === true ? null : "Connect",
		onAction: configured.data === true ? null : () => authenticate.mutate(),
	};
}

function useServiceConnections() {
	return [
		useGCloudConnection(),
		useGitHubConnection(),
		useCodexConnection(),
		useClaudeConnection(),
	];
}

function DashboardLoader() {
	return (
		<>
			<div data-tauri-drag-region className="h-8 shrink-0" />
			<div className="flex flex-1 items-center justify-center px-6">
				<div className="inline-flex items-center gap-2 text-base text-text-muted">
					<Loader />
					<span>Checking connections</span>
				</div>
			</div>
		</>
	);
}

function ServiceRow({ service }: { service: ServiceConnection }) {
	const isActionable =
		!service.connected && service.ready && service.onAction;

	const left = (
		<span className="inline-flex items-center gap-2 text-base text-text-bright">
			{service.icon}
			{service.name}
		</span>
	);

	const right = service.connected ? (
		<span className="inline-flex items-center gap-2 text-base text-text-bright">
			<Check size={14} className="text-[#4ade80]" />
			Connected
		</span>
	) : service.isActing ? (
		<Loader className="text-text-muted" />
	) : isActionable ? (
		<span className="inline-flex items-center gap-1 text-base text-text-muted transition-colors group-hover:text-text-bright">
			Connect
			<ArrowUpRight size={14} />
		</span>
	) : (
		<span className="inline-flex items-center gap-2 text-base text-text-muted">
			<Loader />
			Checking
		</span>
	);

	if (isActionable) {
		return (
			<button
				type="button"
				onClick={service.onAction!}
				disabled={service.isActing}
				data-testid={`onboarding-service-${service.id}`}
				className="group flex items-center justify-between gap-4 rounded-md border border-border-light bg-surface px-4 py-3 transition-colors hover:bg-white/[0.03] disabled:opacity-50"
			>
				{left}
				{right}
			</button>
		);
	}

	return (
		<div
			data-testid={`onboarding-service-${service.id}`}
			className="flex items-center justify-between gap-4 rounded-md border border-border-light bg-surface px-4 py-3"
		>
			{left}
			{right}
		</div>
	);
}

function OnboardingPage({ services }: { services: ServiceConnection[] }) {
	return (
		<>
			<div data-tauri-drag-region className="h-8 shrink-0" />
			<div className="flex flex-1 items-center justify-center px-6">
				<div className="flex w-full max-w-md flex-col gap-4">
					<div className="mb-6 flex justify-center">
						<SiloIcon height={32} />
					</div>
					{services.map((service) => (
						<ServiceRow key={service.id} service={service} />
					))}
				</div>
			</div>
		</>
	);
}

export default function DashboardEntry() {
	const services = useServiceConnections();

	if (services.some((service) => !service.ready)) {
		return <DashboardLoader />;
	}

	if (services.every((service) => service.connected)) {
		return <HomePage />;
	}

	return <OnboardingPage services={services} />;
}
