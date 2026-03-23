import { Check, HardDrive } from "lucide-react";
import type { ReactNode } from "react";
import type { TemplateOperation } from "@/projects/api";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { Wrench } from "lucide-react";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import type { WorkspaceLifecycle } from "@/workspaces/api";

interface Step {
	label: string;
	icon: ReactNode;
	state: "pending" | "active" | "done";
}

const ICON_SIZE = 12;

function StepRow({ step }: { step: Step }) {
	return (
		<div className="flex items-center gap-2.5 text-sm">
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

function ScreenFrame({ steps }: { steps: Step[] }) {
	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="flex flex-col items-center gap-5">
				<SiloIcon height={36} />

				<div className="flex flex-col gap-1.5">
					{steps.map((step) => (
						<StepRow key={step.label} step={step} />
					))}
				</div>
			</div>
		</div>
	);
}

export function TemplateOperationScreen({
	operation,
}: {
	operation: TemplateOperation;
}) {
	const steps =
		operation.kind === "delete"
			? buildDeleteSteps(operation)
			: buildSaveSteps(operation);

	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6 gap-4">
			<ScreenFrame steps={steps} />
			{operation.status === "failed" && operation.last_error ? (
				<p className="max-w-md text-center text-sm text-error">
					{operation.last_error}
				</p>
			) : null}
		</div>
	);
}

export function WorkspaceResumingScreen({
	status,
	lifecycle,
}: {
	status: string;
	lifecycle: WorkspaceLifecycle;
}) {
	const isRunning = status === "RUNNING";
	const resumeState: Step["state"] = isRunning ? "done" : "active";
	const prepareState: Step["state"] =
		!isRunning
			? "pending"
			: lifecycle.phase === "ready"
				? "done"
				: "active";

	return (
		<ScreenFrame
			steps={[
				{
					label: "Resuming virtual machine",
					icon: <GCloudIcon height={ICON_SIZE} />,
					state: resumeState,
				},
				{
					label: "Preparing workspace",
					icon: <Wrench size={ICON_SIZE} />,
					state: prepareState,
				},
			]}
		/>
	);
}

function buildSaveSteps(operation: TemplateOperation): Step[] {
	const completed = operation.status === "completed";

	return [
		{
			label: "Waiting for template workspace",
			icon: <Wrench size={ICON_SIZE} />,
			state: stepState(
				operation,
				["waiting_for_template_ready"],
				["clearing_runtime_state", "stopping_vm", "creating_snapshot", "waiting_for_snapshot_ready", "deleting_old_snapshots", "deleting_template_workspace", "completed"],
				completed,
			),
		},
		{
			label: "Removing runtime state",
			icon: <Wrench size={ICON_SIZE} />,
			state: stepState(
				operation,
				["clearing_runtime_state"],
				["stopping_vm", "creating_snapshot", "waiting_for_snapshot_ready", "deleting_old_snapshots", "deleting_template_workspace", "completed"],
				completed,
			),
		},
		{
			label: "Stopping virtual machine",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: stepState(
				operation,
				["stopping_vm"],
				["creating_snapshot", "waiting_for_snapshot_ready", "deleting_old_snapshots", "deleting_template_workspace", "completed"],
				completed,
			),
		},
		{
			label: "Creating template snapshot",
			icon: <HardDrive size={ICON_SIZE} />,
			state: stepState(
				operation,
				["creating_snapshot", "waiting_for_snapshot_ready"],
				["deleting_old_snapshots", "deleting_template_workspace", "completed"],
				completed,
			),
		},
		{
			label: "Removing previous snapshots",
			icon: <HardDrive size={ICON_SIZE} />,
			state: stepState(
				operation,
				["deleting_old_snapshots"],
				["deleting_template_workspace", "completed"],
				completed,
			),
		},
		{
			label: "Cleaning up template workspace",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: stepState(operation, ["deleting_template_workspace"], ["completed"], completed),
		},
	];
}

function buildDeleteSteps(operation: TemplateOperation): Step[] {
	const completed = operation.status === "completed";

	return [
		{
			label: "Deleting template workspace",
			icon: <GCloudIcon height={ICON_SIZE} />,
			state: stepState(
				operation,
				["deleting_template_workspace"],
				["deleting_snapshots", "completed"],
				completed,
			),
		},
		{
			label: "Deleting template snapshots",
			icon: <HardDrive size={ICON_SIZE} />,
			state: stepState(operation, ["deleting_snapshots"], ["completed"], completed),
		},
	];
}

function stepState(
	operation: TemplateOperation,
	activePhases: string[],
	donePhases: string[],
	completed: boolean,
): Step["state"] {
	if (completed || donePhases.includes(operation.phase)) {
		return "done";
	}

	if (activePhases.includes(operation.phase) || operation.status === "failed") {
		return "active";
	}

	return "pending";
}
