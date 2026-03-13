"use client";

import { useQuery } from "@tanstack/react-query";
import { Check, HardDrive } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { type ReactNode, Suspense, useEffect, useRef, useState } from "react";
import { invoke } from "../../../lib/invoke";
import type { Workspace } from "../../../lib/workspaces";
import { Loader } from "../../components/loader";
import { ChromeIcon } from "../../icons/chrome";
import { GCloudIcon } from "../../icons/gcloud";
import { SiloIcon } from "../../icons/silo";

interface Step {
	label: string;
	icon: ReactNode;
	state: "pending" | "active" | "done";
}

const ICON_SIZE = 12;

function useSavingSteps(status: string, deleted: boolean): Step[] {
	const [wasStopping, setWasStopping] = useState(false);
	const [wasStopped, setWasStopped] = useState(false);

	const isStopping = status === "STOPPING" || status === "SUSPENDING";
	const isStopped = status === "TERMINATED" || status === "STOPPED";

	useEffect(() => {
		if (isStopping) setWasStopping(true);
	}, [isStopping]);

	useEffect(() => {
		if (isStopped) setWasStopped(true);
	}, [isStopped]);

	// 1. Syncing Chrome profile — active until VM starts stopping
	const chromeState: Step["state"] = wasStopping || isStopped
		? "done"
		: "active";

	// 2. Stopping virtual machine — active while stopping, done once stopped
	const stopState: Step["state"] = isStopped || wasStopped
		? "done"
		: isStopping
			? "active"
			: "pending";

	// 3. Snapshotting disk — active once stopped, done when VM is deleted
	const snapshotState: Step["state"] = deleted
		? "done"
		: wasStopped
			? "active"
			: "pending";

	// 4. Cleaning up — active after snapshot (VM deletion), done when complete
	const cleanupState: Step["state"] = deleted
		? "done"
		: snapshotState === "done"
			? "active"
			: "pending";

	return [
		{ label: "Syncing Chrome profile", icon: <ChromeIcon height={ICON_SIZE} />, state: chromeState },
		{ label: "Stopping virtual machine", icon: <GCloudIcon height={ICON_SIZE} />, state: stopState },
		{ label: "Snapshotting disk", icon: <HardDrive size={ICON_SIZE} />, state: snapshotState },
		{ label: "Cleaning up", icon: <GCloudIcon height={ICON_SIZE} />, state: cleanupState },
	];
}

function StepRow({ step }: { step: Step }) {
	return (
		<div className="flex items-center gap-2.5 text-[11px]">
			<span className={`w-3 flex items-center justify-center shrink-0 ${step.state === "pending" ? "opacity-30" : ""}`}>
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
	return (
		<Suspense>
			<SavingView />
		</Suspense>
	);
}

function SavingView() {
	const searchParams = useSearchParams();
	const router = useRouter();
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
		const timer = setTimeout(() => router.push("/"), 1500);
		return () => clearTimeout(timer);
	}, [deleted, router]);

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
