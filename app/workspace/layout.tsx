"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, Terminal, X } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo } from "react";
import { CloudDeck, useCloud } from "../../components/cloud";
import { ClaudeIcon } from "../../components/icons/claude";
import { CodexIcon } from "../../components/icons/codex";
import { Loader } from "../../components/loader";
import { toast } from "../../components/toaster";
import { TopBar } from "../../components/top-bar";
import { cloudSessionHref, normalizeTerminalSession } from "../../lib/cloud";
import { invoke } from "../../lib/invoke";
import type { Workspace, WorkspaceSession } from "../../lib/workspaces";

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
			<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
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

function terminalTabPresentation(name: string) {
	const trimmed = name.trim();
	const lower = trimmed.toLowerCase();
	const [token, ...rest] = trimmed.split(/\s+/);
	const normalizedToken = token?.toLowerCase() ?? "";
	const normalizedAssistant =
		normalizedToken === "silo"
			? (rest[0]?.toLowerCase() ?? "")
			: normalizedToken;
	if (normalizedAssistant === "cc" || normalizedAssistant === "claude") {
		return {
			icon: <ClaudeIcon height={12} />,
			label: "claude",
		};
	}
	if (normalizedAssistant === "codex" || lower.startsWith("command codex")) {
		return {
			icon: <CodexIcon height={12} />,
			label: "codex",
		};
	}
	return {
		icon: <Terminal size={12} />,
		label: trimmed || "shell",
	};
}

function WorkspaceLayoutInner({ children }: { children: React.ReactNode }) {
	const searchParams = useSearchParams();
	const router = useRouter();
	const queryClient = useQueryClient();
	const { ensureWorkspaceSessions, removeSession } = useCloud();
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
	const isWorkspaceReady =
		workspace.data?.status === "RUNNING" && workspace.data?.ready === true;

	const terminals = useQuery({
		queryKey: ["terminal_list_terminals", workspaceName],
		queryFn: () =>
			invoke<WorkspaceSession[]>(
				"terminal_list_terminals",
				{ workspace: workspaceName },
				{
					log: "state_changes_only",
					key: `poll:terminal_list_terminals:${workspaceName}`,
				},
			),
		enabled: !!workspaceName && isWorkspaceReady,
		refetchInterval: 2000,
	});

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ attachment_id: string }>("terminal_create_terminal", {
				workspace: workspaceName,
			}),
		onSuccess: (result) => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspaceName],
			});
			router.push(
				cloudSessionHref({
					project,
					workspace: workspaceName,
					kind: "terminal",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
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

	const project = searchParams.get("project") ?? workspace.data?.project ?? "";
	const activeKind = searchParams.get("kind");
	const activeAttachmentId = searchParams.get("attachment_id");
	const fresh = searchParams.get("fresh") === "1";
	const workspaceHref = useMemo(() => {
		const params = new URLSearchParams({ name: workspaceName });
		if (project) {
			params.set("project", project);
		}
		return `/workspace?${params.toString()}`;
	}, [project, workspaceName]);
	const terminalData = terminals.data;
	const terminalList = terminalData ?? [];
	const cloudSessions = useMemo(
		() =>
			(terminalData ?? []).map((session) =>
				normalizeTerminalSession(workspaceName, session),
			),
		[terminalData, workspaceName],
	);
	const activeSession =
		workspaceName && activeKind === "terminal" && activeAttachmentId
			? (cloudSessions.find(
					(session) => session.attachmentId === activeAttachmentId,
				) ?? {
					workspace: workspaceName,
					kind: "terminal",
					attachmentId: activeAttachmentId,
					name: activeAttachmentId,
					working: null,
					unread: null,
				})
			: null;
	const showCloudDeck = activeSession?.kind === "terminal";

	useEffect(() => {
		if (!workspaceName || !isWorkspaceReady) {
			return;
		}

		ensureWorkspaceSessions(workspaceName, cloudSessions);
	}, [cloudSessions, ensureWorkspaceSessions, isWorkspaceReady, workspaceName]);

	return (
		<>
			{workspace.data ? (
				<TopBar workspace={workspace.data} />
			) : (
				<header className="h-9 w-full border-b border-border-light shrink-0 flex items-center relative">
					<div data-tauri-drag-region className="absolute inset-0" />
					<div className="relative flex items-center gap-1.5 px-3 z-10">
						<div className="h-3 w-20 rounded bg-border-light animate-pulse" />
						<div className="h-3 w-16 rounded bg-border-light animate-pulse" />
					</div>
				</header>
			)}
			{terminalList.length > 0 && (
				<div className="w-full bg-bg shrink-0 flex items-end overflow-x-auto">
					{terminalList.map((session) => (
						<TerminalTab
							key={session.attachment_id}
							session={session}
							isActive={
								activeKind === "terminal" &&
								activeAttachmentId === session.attachment_id
							}
							workspaceName={workspaceName}
							project={project}
							workspaceHref={workspaceHref}
						/>
					))}
					<button
						type="button"
						disabled={createTerminal.isPending}
						onClick={() => createTerminal.mutate()}
						className="h-9 flex items-center px-2.5 border-b border-border-light text-text-muted hover:text-text-bright transition-colors disabled:opacity-50"
					>
						{createTerminal.isPending ? <Loader /> : <Plus size={12} />}
					</button>
					<div className="flex-1 h-9 border-b border-border-light" />
				</div>
			)}
			{showCloudDeck ? (
				<CloudDeck
					workspace={workspaceName}
					activeSession={activeSession}
					skipInitialScrollback={fresh}
				/>
			) : (
				children
			)}
		</>
	);
}

function TerminalTab({
	session,
	isActive,
	workspaceName,
	project,
	workspaceHref,
}: {
	session: WorkspaceSession;
	isActive: boolean;
	workspaceName: string;
	project: string;
	workspaceHref: string;
}) {
	const router = useRouter();
	const queryClient = useQueryClient();
	const { removeSession } = useCloud();

	const killTerminal = useMutation({
		mutationFn: () =>
			invoke("terminal_kill_terminal", {
				workspace: workspaceName,
				attachmentId: session.attachment_id,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["terminal_list_terminals", workspaceName],
			});
			removeSession(workspaceName, "terminal", session.attachment_id);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to close terminal",
				description: error.message,
			});
		},
	});

	const handleClose = () => {
		if (isActive) {
			router.replace(workspaceHref);
		}
		killTerminal.mutate();
	};

	const { icon, label } = terminalTabPresentation(session.name);

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: can't use <button> because it contains interactive children
		<div
			onClick={() =>
				router.push(
					cloudSessionHref({
						project,
						workspace: workspaceName,
						kind: "terminal",
						attachmentId: session.attachment_id,
					}),
				)
			}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					router.push(
						cloudSessionHref({
							project,
							workspace: workspaceName,
							kind: "terminal",
							attachmentId: session.attachment_id,
						}),
					);
				}
			}}
			className={`group/tab h-9 flex items-center gap-1.5 pl-3.5 pr-2.5 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
				isActive
					? "bg-surface text-text-bright border-r-border-light border-b-surface"
					: "text-text border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			{icon}
			<span className="max-w-48 truncate">{label}</span>
			{killTerminal.isPending ? (
				<span className="p-0.5">
					<Loader className="text-error" />
				</span>
			) : session.working ? (
				<span className="p-0.5">
					<Loader className="text-blue-400" />
				</span>
			) : session.unread && !isActive ? (
				<span className="group/unread relative p-0.5 flex items-center justify-center">
					<span className="shrink-0 w-2 h-2 rounded-full bg-blue-400 group-hover/unread:hidden" />
					<button
						type="button"
						onClick={(e) => {
							e.stopPropagation();
							handleClose();
						}}
						className="hidden group-hover/unread:block p-0.5 rounded transition-colors hover:bg-border-light text-text-muted hover:text-text-bright"
					>
						<X size={10} />
					</button>
				</span>
			) : (
				<button
					type="button"
					onClick={(e) => {
						e.stopPropagation();
						handleClose();
					}}
					className={`p-0.5 rounded transition-colors hover:bg-border-light ${
						isActive
							? "text-text-muted hover:text-text-bright"
							: "opacity-0 group-hover/tab:opacity-100 text-text-muted hover:text-text-bright"
					}`}
				>
					<X size={10} />
				</button>
			)}
		</div>
	);
}
