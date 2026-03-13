"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { TopBar } from "../components/top-bar";
import { invoke } from "../../lib/invoke";
import { type Workspace } from "../../lib/workspaces";

export default function WorkspaceLayout({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	return (
		<Suspense fallback={<LayoutSkeleton>{children}</LayoutSkeleton>}>
			<WorkspaceLayoutInner>{children}</WorkspaceLayoutInner>
		</Suspense>
	);
}

function LayoutSkeleton({ children }: { children: React.ReactNode }) {
	return (
		<>
			<header className="h-8 w-full border-b border-border-light shrink-0 flex items-center relative">
				<div data-tauri-drag-region className="absolute inset-0" />
				<div className="relative flex items-center gap-1.5 px-3 z-10">
					<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
					<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
				</div>
			</header>
			{children}
		</>
	);
}

function WorkspaceLayoutInner({
	children,
}: { children: React.ReactNode }) {
	const searchParams = useSearchParams();
	const workspaceName =
		searchParams.get("name") ?? searchParams.get("workspace") ?? "";

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

	return (
		<>
			{workspace.data ? (
				<TopBar workspace={workspace.data} />
			) : (
				<header className="h-8 w-full border-b border-border-light shrink-0 flex items-center relative">
					<div data-tauri-drag-region className="absolute inset-0" />
					<div className="relative flex items-center gap-1.5 px-3 z-10">
						<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
						<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
					</div>
				</header>
			)}
			{children}
		</>
	);
}
