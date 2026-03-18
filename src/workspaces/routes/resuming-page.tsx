"use client";

import { useQuery } from "@tanstack/react-query";
import { Check } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { GCloudIcon } from "@/shared/ui/icons/gcloud";
import { SiloIcon } from "@/shared/ui/icons/silo";
import { Loader } from "@/shared/ui/loader";
import { invoke } from "@/shared/lib/invoke";
import type { Workspace } from "@/workspaces/api";

interface Step {
	label: string;
	icon: ReactNode;
	state: "pending" | "active" | "done";
}

const ICON_SIZE = 12;

function useResumingSteps(status: string, ready: boolean): Step[] {
	const isRunning = status === "RUNNING";

	// 1. Resuming virtual machine — active until RUNNING, done when RUNNING
	const resumeState: Step["state"] = isRunning ? "done" : "active";

	// 2. Preparing workspace — active when RUNNING but not ready, done when ready
	const prepareState: Step["state"] = isRunning
		? ready
			? "done"
			: "active"
		: "pending";

	return [
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
	];
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

export default function ResumingPage() {
	return <ResumingView />;
}

function ResumingView() {
	const [searchParams] = useSearchParams();
	const navigate = useNavigate();
	const workspaceName = searchParams.get("workspace") ?? "";
	const projectName = searchParams.get("project") ?? "";
	const routedRef = useRef(false);

	const workspace = useQuery({
		queryKey: ["workspaces_get_workspace", workspaceName],
		queryFn: () =>
			invoke<Workspace>(
				"workspaces_get_workspace",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:workspaces_get_workspace:${workspaceName}`,
				},
			),
		enabled: !!workspaceName,
		refetchInterval: 2000,
	});

	const status = workspace.data?.status ?? "";
	const ready = workspace.data?.ready ?? false;
	const steps = useResumingSteps(status, ready);

	// Once workspace is RUNNING and ready, wait briefly then route to workspace
	useEffect(() => {
		if (status !== "RUNNING" || !ready || routedRef.current) return;
		routedRef.current = true;
		const timer = setTimeout(
			() =>
				navigate(
					`/workspace?project=${encodeURIComponent(projectName)}&name=${encodeURIComponent(workspaceName)}`,
				),
			500,
		);
		return () => clearTimeout(timer);
	}, [status, ready, navigate, projectName, workspaceName]);

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
