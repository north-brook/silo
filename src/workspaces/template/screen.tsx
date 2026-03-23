import { Check, Terminal } from "lucide-react";
import type { ReactNode } from "react";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { GHIcon } from "@/shared/ui/icons/gh";
import { LogoIcon } from "@/shared/ui/icons/logo";
import { Loader } from "@/shared/ui/loader";
import type { WorkspaceLifecycle } from "@/workspaces/api";

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
		phase === "bootstrapping" ||
		phase === "waiting_for_agent" ||
		phase === "starting_terminal" ||
		phase === "ready"
			? phase === "bootstrapping"
				? "active"
				: "done"
			: "pending";
	const secureAccessState: Step["state"] =
		phase === "waiting_for_agent"
			? "active"
			: phase === "starting_terminal" || phase === "ready"
				? "done"
				: "pending";
	const terminalState: Step["state"] =
		phase === "starting_terminal"
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
							phase === "starting_terminal" ||
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
		{
			label: "Opening terminal session",
			icon: <Terminal size={ICON_SIZE} />,
			state: terminalState,
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
}: {
	lifecycle: WorkspaceLifecycle;
	status: string;
}) {
	const { steps, allDone } = useProvisioningSteps(status, lifecycle);
	const startupFailed = lifecycle.phase === "failed";
	const startupError =
		lifecycle.last_error ?? lifecycle.detail ?? "Template startup failed";

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="flex flex-col items-center gap-5">
				<LogoIcon height={24} className="opacity-40" />

				<div className="flex flex-col gap-1.5">
					{steps.map((step) => (
						<StepRow key={step.label} step={step} />
					))}
				</div>

				{startupFailed ? (
					<p className="max-w-md text-center text-[11px] text-error">
						{startupError}
					</p>
				) : allDone ? (
					<div className="flex items-center gap-2 text-[11px] text-text-muted">
						<Loader className="text-text-muted" />
						<span>Opening terminal...</span>
					</div>
				) : null}
			</div>
		</div>
	);
}
