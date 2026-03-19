import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Globe, Terminal } from "lucide-react";
import { type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { GHIcon } from "@/shared/ui/icons/gh";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import { invoke } from "@/shared/lib/invoke";
import { type WorkspaceLifecycle } from "@/workspaces/api";
import {
	type SessionRouteState,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";

interface Step {
	label: string;
	icon: ReactNode;
	state: "pending" | "active" | "done";
}

type ConfigStep = {
	label: string;
	icon: ReactNode;
};

const ICON_SIZE = 12;
const CONFIG_STEPS: ConfigStep[] = [
	{
		label: "Configuring git",
		icon: <GHIcon height={ICON_SIZE} />,
	},
	{
		label: "Configuring codex",
		icon: <CodexIcon height={ICON_SIZE} />,
	},
	{
		label: "Configuring claude code",
		icon: <ClaudeIcon height={ICON_SIZE} />,
	},
];

function useProvisioningSteps(
	status: string,
	lifecycle: WorkspaceLifecycle,
): { steps: Step[]; allDone: boolean } {
	const isRunning = status === "RUNNING";
	const phase = lifecycle.phase;

	const vmProvisionState: Step["state"] = isRunning
		? "done"
		: status === "STAGING" || status === "PROVISIONING"
			? "active"
			: "pending";
	const configState: Step["state"] =
		phase === "bootstrapping" || phase === "waiting_for_agent" || phase === "ready"
			? phase === "bootstrapping"
				? "active"
				: "done"
			: phase === "waiting_for_ssh"
				? "pending"
				: "pending";
	const secureAccessState: Step["state"] =
		phase === "waiting_for_agent"
			? "active"
			: phase === "ready"
				? "done"
				: "pending";

	const steps: Step[] = [
		{
			label: "Provisioning virtual machine",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: vmProvisionState,
		},
		{
			label: "Waiting for SSH",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state:
				phase === "waiting_for_ssh"
					? "active"
					: phase === "bootstrapping" ||
					  phase === "waiting_for_agent" ||
						  phase === "ready"
						? "done"
						: vmProvisionState === "done"
							? "active"
							: "pending",
		},
		...CONFIG_STEPS.map(({ label, icon }) => ({
			label,
			icon,
			state: configState,
		})),
		{
			label: "Configuring secure access",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: secureAccessState,
		},
	];

	const allDone = phase === "ready";

	return { steps, allDone };
}

function StepRow({ step }: { step: Step }) {
	return (
		<div className="flex items-center gap-2.5 text-[11px]">
			<span
				className={`w-3 flex items-center justify-center shrink-0 ${step.state === "pending" ? "opacity-30" : ""}`}
			>
				{step.icon}
			</span>
			<span
				className={
					step.state === "done"
						? "text-text-muted"
						: step.state === "active"
							? "text-text"
							: "text-text-placeholder"
				}
			>
				{step.label}
			</span>
			<span className="ml-auto w-3 flex items-center justify-center shrink-0">
				{step.state === "done" ? (
					<Check size={10} className="text-green-500" />
				) : step.state === "active" ? (
					<Loader />
				) : null}
			</span>
		</div>
	);
}

export function TemplatingWorkspace({
	lifecycle,
	status,
	workspace,
	project,
}: {
	isRunning: boolean;
	lifecycle: WorkspaceLifecycle;
	status: string;
	workspace: string;
	project: string | null;
}) {
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const { steps, allDone } = useProvisioningSteps(status, lifecycle);
	const startupFailed = lifecycle.phase === "failed";
	const startupError =
		lifecycle.last_error ?? lifecycle.detail ?? "Template startup failed";

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ attachment_id: string }>("terminal_create_terminal", {
				workspace,
			}),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspace],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
			navigate(
				workspaceSessionHref({
					project: project ?? "",
					workspace,
					kind: "terminal",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create terminal",
				description: error.message,
			});
		},
	});

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="flex flex-col items-center gap-5">
				<SiloIcon height={24} className="opacity-40" />

				{!allDone && (
					<div className="flex flex-col gap-1.5">
						{steps.map((step) => (
							<StepRow key={step.label} step={step} />
						))}
					</div>
				)}

				{startupFailed ? (
					<p className="max-w-md text-center text-[11px] text-error">
						{startupError}
					</p>
				) : null}

				{allDone && (
					<div className="flex items-center gap-3">
						<button
							type="button"
							disabled={createTerminal.isPending}
							onClick={() => createTerminal.mutate()}
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							{createTerminal.isPending ? <Loader /> : <Terminal size={12} />}
							Open Terminal
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							<Globe size={12} />
							Open Browser
						</button>
					</div>
				)}
			</div>
		</div>
	);
}
