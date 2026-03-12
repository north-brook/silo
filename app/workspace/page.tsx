"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Play, Square, Trash2 } from "lucide-react";
import { TopBar } from "../components/top-bar";
import { toast } from "../components/toaster";
import { invoke } from "../../lib/invoke";

interface Workspace {
	name: string;
	project: string | null;
	branch: string | null;
	working: boolean | null;
	last_active: string | null;
	created_at: string;
	status: string;
	zone: string;
}

export default function WorkspacePage() {
	return (
		<Suspense>
			<WorkspaceView />
		</Suspense>
	);
}

function WorkspaceView() {
	const searchParams = useSearchParams();
	const project = searchParams.get("project") ?? "";
	const workspaceName = searchParams.get("name") ?? "";
	const queryClient = useQueryClient();

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
		refetchInterval: 10000,
	});

	const start = useMutation({
		mutationFn: () =>
			invoke("workspaces_start_workspace", { workspace: workspaceName }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces", project],
			});
			toast({ variant: "success", title: "Workspace started" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to start workspace",
				description: error.message,
			});
		},
	});

	const stop = useMutation({
		mutationFn: () =>
			invoke("workspaces_stop_workspace", { workspace: workspaceName }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
			});
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces", project],
			});
			toast({ variant: "success", title: "Workspace stopped" });
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to stop workspace",
				description: error.message,
			});
		},
	});

	const remove = useMutation({
		mutationFn: () =>
			invoke("workspaces_delete_workspace", { workspace: workspaceName }),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_list_workspaces", project],
			});
			toast({ variant: "success", title: "Workspace deleted" });
			window.history.back();
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to delete workspace",
				description: error.message,
			});
		},
	});

	const isRunning = workspace.data?.status === "RUNNING";
	const isStopped =
		workspace.data?.status === "TERMINATED" ||
		workspace.data?.status === "STOPPED";
	const isPending = start.isPending || stop.isPending || remove.isPending;

	return (
		<>
			<TopBar />
			<div className="flex-1 overflow-auto p-4">
				<div className="flex items-center justify-between mb-4">
					<div>
						<h2 className="text-sm">{workspaceName}</h2>
						<span className="text-xs text-text-muted">{project}</span>
					</div>
					<div className="flex items-center gap-2">
						{isStopped && (
							<button
								type="button"
								onClick={() => start.mutate()}
								disabled={isPending}
								className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-btn border border-border-light text-text-bright hover:bg-btn-hover hover:border-border-hover transition-colors disabled:opacity-50"
							>
								<Play size={12} />
								Start
							</button>
						)}
						{isRunning && (
							<button
								type="button"
								onClick={() => stop.mutate()}
								disabled={isPending}
								className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-btn border border-border-light text-text-bright hover:bg-btn-hover hover:border-border-hover transition-colors disabled:opacity-50"
							>
								<Square size={12} />
								Stop
							</button>
						)}
						<button
							type="button"
							onClick={() => remove.mutate()}
							disabled={isPending}
							className="flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-btn border border-error/20 text-error hover:bg-error/5 transition-colors disabled:opacity-50"
						>
							<Trash2 size={12} />
							Delete
						</button>
					</div>
				</div>

				{workspace.isLoading && (
					<span className="text-xs text-text-muted">Loading...</span>
				)}

				{workspace.data && (
					<div className="grid grid-cols-2 gap-x-6 gap-y-2 text-xs">
						<span className="text-text-muted">Status</span>
						<span className={isRunning ? "text-success" : "text-text"}>
							{workspace.data.status}
						</span>

						<span className="text-text-muted">Zone</span>
						<span>{workspace.data.zone}</span>

						{workspace.data.branch && (
							<>
								<span className="text-text-muted">Branch</span>
								<span>{workspace.data.branch}</span>
							</>
						)}

						<span className="text-text-muted">Created</span>
						<span>{workspace.data.created_at}</span>

						{workspace.data.last_active && (
							<>
								<span className="text-text-muted">Last active</span>
								<span>{workspace.data.last_active}</span>
							</>
						)}
					</div>
				)}

				{workspace.isError && (
					<span className="text-xs text-error">{workspace.error.message}</span>
				)}
			</div>
		</>
	);
}
