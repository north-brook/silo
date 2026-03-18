"use client";

import { useQuery } from "@tanstack/react-query";
import { Check, HardDrive } from "lucide-react";
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

function useSavingSteps(status: string, deleted: boolean): Step[] {
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

	return [
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

export default function SavingPage() {
	return <SavingView />;
}

function SavingView() {
	const [searchParams] = useSearchParams();
	const navigate = useNavigate();
	const workspaceName = searchParams.get("workspace") ?? "";
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
	const deleted = workspace.isError || (workspace.isSuccess && !workspace.data);
	const steps = useSavingSteps(status, deleted);

	// Once the workspace is deleted (save complete), wait a moment then route home
	useEffect(() => {
		if (!deleted || routedRef.current) return;
		routedRef.current = true;
		const timer = setTimeout(() => navigate("/"), 1500);
		return () => clearTimeout(timer);
	}, [deleted, navigate]);

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
