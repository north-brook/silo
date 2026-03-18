import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check, Globe, Terminal } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { GHIcon } from "@/shared/ui/icons/gh";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import { invoke } from "@/shared/lib/invoke";
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
	delay: number;
};

const ICON_SIZE = 12;
const CONFIG_STEPS: ConfigStep[] = [
	{
		label: "Configuring git",
		icon: <GHIcon height={ICON_SIZE} />,
		delay: 2_000,
	},
	{
		label: "Configuring codex",
		icon: <CodexIcon height={ICON_SIZE} />,
		delay: 2_000,
	},
	{
		label: "Configuring claude code",
		icon: <ClaudeIcon height={ICON_SIZE} />,
		delay: 2_000,
	},
	{
		label: "Configuring chrome",
		icon: <Globe size={ICON_SIZE} />,
		delay: 30_000,
	},
];

function useProvisioningSteps(
	status: string,
	ready: boolean,
): { steps: Step[]; allDone: boolean } {
	const [configIndex, setConfigIndex] = useState(-1);

	const isRunning = status === "RUNNING";
	const isProvisioning = status === "STAGING" || status === "PROVISIONING";

	// VM-derived step states
	const vmProvisionState: Step["state"] = isRunning
		? "done"
		: isProvisioning
			? "active"
			: "pending";
	const vmStartState: Step["state"] = isRunning
		? "done"
		: vmProvisionState === "done"
			? "active"
			: "pending";

	// Start config timers once VM is running
	useEffect(() => {
		if (!isRunning) return;
		setConfigIndex(0);

		const timers: ReturnType<typeof setTimeout>[] = [];
		let cumulative = 0;
		for (let i = 0; i < CONFIG_STEPS.length; i++) {
			cumulative += CONFIG_STEPS[i].delay;
			timers.push(setTimeout(() => setConfigIndex(i + 1), cumulative));
		}

		return () => {
			for (const t of timers) clearTimeout(t);
		};
	}, [isRunning]);

	// "Configuring secure access" runs until ready
	const configsDone = configIndex >= CONFIG_STEPS.length;
	const secureAccessState: Step["state"] = ready
		? "done"
		: configsDone
			? "active"
			: "pending";

	const steps: Step[] = [
		{
			label: "Provisioning virtual machine",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: vmProvisionState,
		},
		{
			label: "Starting virtual machine",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: vmStartState,
		},
		...CONFIG_STEPS.map(({ label, icon }, i) => {
			let state: Step["state"] = ready ? "done" : "pending";
			if (!ready) {
				if (configIndex > i) state = "done";
				else if (configIndex === i) state = "active";
			}
			return { label, icon, state };
		}),
		{
			label: "Configuring secure access",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: secureAccessState,
		},
	];

	const allDone = ready;

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
	ready,
	status,
	workspace,
	project,
}: {
	isRunning: boolean;
	ready: boolean;
	status: string;
	workspace: string;
	project: string | null;
}) {
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const { steps, allDone } = useProvisioningSteps(status, ready);

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
