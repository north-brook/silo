import { Check, HardDrive } from "lucide-react";
import type { ReactNode } from "react";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
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

function ScreenFrame({ steps }: { steps: Step[] }) {
	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="flex flex-col items-center gap-5">
				<SiloIcon height={24} className="opacity-40" />

				<div className="flex flex-col gap-1.5">
					{steps.map((step) => (
						<StepRow key={step.label} step={step} />
					))}
				</div>
			</div>
		</div>
	);
}

export function WorkspaceSavingScreen({
	status,
	deleted,
}: {
	status: string;
	deleted: boolean;
}) {
	const isStopping = status === "STOPPING";
	const isStopped = status === "TERMINATED" || status === "STOPPED";
	const stopState: Step["state"] =
		deleted || isStopped ? "done" : isStopping ? "active" : "pending";
	const snapshotState: Step["state"] = deleted
		? "done"
		: isStopped
			? "active"
			: "pending";
	const cleanupState: Step["state"] = deleted ? "done" : "pending";

	return (
		<ScreenFrame
			steps={[
				{
					label: "Stopping virtual machine",
					icon: <GCloudIcon height={ICON_SIZE} />,
					state: stopState,
				},
				{
					label: "Snapshotting disk",
					icon: <HardDrive size={ICON_SIZE} />,
					state: snapshotState,
				},
				{
					label: "Cleaning up",
					icon: <GCloudIcon height={ICON_SIZE} />,
					state: cleanupState,
				},
			]}
		/>
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
					icon: <SiloIcon height={ICON_SIZE} />,
					state: prepareState,
				},
			]}
		/>
	);
}
