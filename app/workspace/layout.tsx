"use client";

import {
	useMutation,
	useMutationState,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { isTauri } from "@tauri-apps/api/core";
import { Globe, Plus, Terminal, X } from "lucide-react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { useCloud } from "../../components/cloud";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "../../components/dialog";
import { ClaudeIcon } from "../../components/icons/claude";
import { CodexIcon } from "../../components/icons/codex";
import { Loader } from "../../components/loader";
import { toast } from "../../components/toaster";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../components/tooltip";
import { TopBar } from "../../components/top-bar";
import { cloudSessionHref, normalizeWorkspaceSession } from "../../lib/cloud";
import { invoke } from "../../lib/invoke";
import { listenShortcutEvent, shortcutEvents } from "../../lib/shortcuts";
import {
	isTemplateWorkspace,
	type Workspace,
	type WorkspaceSession,
	workspaceSessions,
} from "../../lib/workspaces";

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

function browserTabPresentation(session: WorkspaceSession) {
	const label =
		session.title?.trim() ||
		session.name.trim() ||
		session.url?.trim() ||
		"browser";
	const icon = session.favicon_url ? (
		<span
			aria-hidden
			className="h-3 w-3 rounded-[2px] shrink-0 bg-center bg-cover bg-no-repeat"
			style={{ backgroundImage: `url("${session.favicon_url}")` }}
		/>
	) : (
		<Globe size={12} />
	);
	return { icon, label };
}

function findLiveNeighbor(
	sessions: WorkspaceSession[],
	closingIndex: number,
	deletingIds: ReadonlySet<string>,
): WorkspaceSession | null {
	for (let i = closingIndex - 1; i >= 0; i--) {
		if (!deletingIds.has(sessions[i].attachment_id)) return sessions[i];
	}
	for (let i = closingIndex + 1; i < sessions.length; i++) {
		if (!deletingIds.has(sessions[i].attachment_id)) return sessions[i];
	}
	return null;
}

function WorkspaceLayoutInner({ children }: { children: React.ReactNode }) {
	const searchParams = useSearchParams();
	const pathname = usePathname();
	const router = useRouter();
	const queryClient = useQueryClient();
	const { ensureWorkspaceSessions, removeSession } = useCloud();
	const [newTabOpen, setNewTabOpen] = useState(false);
	const workspaceName =
		searchParams.get("name") ?? searchParams.get("workspace") ?? "";
	const activeKind = searchParams.get("kind");
	const activeAttachmentId = searchParams.get("attachment_id");
	const projectParam = searchParams.get("project") ?? "";

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
	const isWorkspaceReady =
		workspace.data?.status === "RUNNING" && workspace.data?.ready === true;
	const project = projectParam || workspace.data?.project || "";
	const workspaceHref = useMemo(() => {
		const params = new URLSearchParams({ name: workspaceName });
		if (project) {
			params.set("project", project);
		}
		return `/workspace?${params.toString()}`;
	}, [project, workspaceName]);

	const isCurrentLayoutInstance = useCallback(() => {
		if (typeof window === "undefined") {
			return true;
		}
		const currentUrl = new URL(window.location.href);
		const currentWorkspace =
			currentUrl.searchParams.get("name") ??
			currentUrl.searchParams.get("workspace") ??
			"";
		const currentProject = currentUrl.searchParams.get("project") ?? "";
		const currentKind = currentUrl.searchParams.get("kind");
		const currentAttachmentId = currentUrl.searchParams.get("attachment_id");

		if (currentUrl.pathname !== pathname) {
			return false;
		}
		if (currentWorkspace !== workspaceName || currentProject !== project) {
			return false;
		}
		if (activeKind !== currentKind) {
			return false;
		}
		if ((activeAttachmentId ?? null) !== currentAttachmentId) {
			return false;
		}
		return true;
	}, [activeAttachmentId, activeKind, pathname, project, workspaceName]);

	const sessions = useMemo<WorkspaceSession[]>(
		() =>
			workspace.data && !isTemplateWorkspace(workspace.data)
				? workspaceSessions(workspace.data)
				: [],
		[workspace.data],
	);
	const cloudSessions = useMemo(
		() =>
			sessions.map((session) =>
				normalizeWorkspaceSession(workspaceName, session),
			),
		[sessions, workspaceName],
	);
	useEffect(() => {
		if (!workspaceName || !isWorkspaceReady) {
			return;
		}
		ensureWorkspaceSessions(workspaceName, cloudSessions);
	}, [cloudSessions, ensureWorkspaceSessions, isWorkspaceReady, workspaceName]);

	const createTerminal = useMutation({
		mutationFn: () =>
			invoke<{ attachment_id: string }>("terminal_create_terminal", {
				workspace: workspaceName,
			}),
		onSuccess: (result) => {
			setNewTabOpen(false);
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
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

	const createAssistant = useMutation({
		mutationFn: (model: "codex" | "claude") =>
			invoke<{ attachment_id: string }>("terminal_create_assistant", {
				workspace: workspaceName,
				model,
			}),
		onSuccess: (result) => {
			setNewTabOpen(false);
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
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
				title: "Failed to create assistant",
				description: error.message,
			});
		},
	});

	const createBrowser = useMutation({
		mutationFn: () =>
			invoke<{ attachment_id: string }>("browser_create_tab", {
				workspace: workspaceName,
			}),
		onSuccess: (result) => {
			setNewTabOpen(false);
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
			});
			router.push(
				cloudSessionHref({
					project,
					workspace: workspaceName,
					kind: "browser",
					attachmentId: result.attachment_id,
					fresh: true,
				}),
			);
		},
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to create browser",
				description: error.message,
			});
		},
	});

	const isPending =
		createTerminal.isPending ||
		createAssistant.isPending ||
		createBrowser.isPending;

	const killSession = useMutation({
		mutationKey: ["kill-session", workspaceName],
		mutationFn: (session: WorkspaceSession) =>
			session.type === "browser"
				? invoke("browser_kill_tab", {
						workspace: workspaceName,
						attachmentId: session.attachment_id,
					})
				: invoke("terminal_kill_terminal", {
						workspace: workspaceName,
						attachmentId: session.attachment_id,
					}),
		onSuccess: (_, session) => {
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspaceName],
			});
			removeSession(workspaceName, session.type, session.attachment_id);
		},
		onError: (error, session) => {
			toast({
				variant: "error",
				title:
					session.type === "browser"
						? "Failed to close browser"
						: "Failed to close terminal",
				description: error.message,
			});
		},
	});

	const pendingKills = useMutationState({
		filters: {
			mutationKey: ["kill-session", workspaceName],
			status: "pending",
		},
		select: (mutation) =>
			(mutation.state.variables as WorkspaceSession | undefined)?.attachment_id,
	});
	const deletingIds = useMemo(
		() => new Set(pendingKills.filter((id): id is string => !!id)),
		[pendingKills],
	);

	const closeTab = useCallback(
		(session: WorkspaceSession) => {
			if (deletingIds.has(session.attachment_id)) return;

			const isActive =
				activeKind === session.type &&
				activeAttachmentId === session.attachment_id;
			if (isActive) {
				const idx = sessions.findIndex(
					(s) => s.attachment_id === session.attachment_id,
				);
				// Build a preview set including the tab we're about to close
				const preview = new Set(deletingIds);
				preview.add(session.attachment_id);
				const neighbor = findLiveNeighbor(sessions, idx, preview);
				if (neighbor) {
					router.replace(
						cloudSessionHref({
							project,
							workspace: workspaceName,
							kind: neighbor.type,
							attachmentId: neighbor.attachment_id,
						}),
					);
				} else {
					router.replace(workspaceHref);
				}
			}

			killSession.mutate(session);
		},
		[
			deletingIds,
			activeKind,
			activeAttachmentId,
			sessions,
			router,
			project,
			workspaceName,
			workspaceHref,
			killSession,
		],
	);

	const closeActiveTab = useCallback(() => {
		if (!isCurrentLayoutInstance()) return;
		if (!activeKind || !activeAttachmentId) return;
		const session = sessions.find(
			(s) => s.type === activeKind && s.attachment_id === activeAttachmentId,
		);
		if (!session) return;
		closeTab(session);
	}, [
		sessions,
		activeKind,
		activeAttachmentId,
		closeTab,
		isCurrentLayoutInstance,
	]);

	const navigateToPreviousTab = useCallback(() => {
		if (!isCurrentLayoutInstance()) return;
		const liveSessions = sessions.filter(
			(s) => !deletingIds.has(s.attachment_id),
		);
		if (liveSessions.length === 0) return;
		const activeIndex = liveSessions.findIndex(
			(s) => s.type === activeKind && s.attachment_id === activeAttachmentId,
		);
		if (activeIndex === -1) return;
		const prevIndex =
			activeIndex === 0 ? liveSessions.length - 1 : activeIndex - 1;
		const prev = liveSessions[prevIndex];
		router.push(
			cloudSessionHref({
				project,
				workspace: workspaceName,
				kind: prev.type,
				attachmentId: prev.attachment_id,
			}),
		);
	}, [
		sessions,
		deletingIds,
		activeKind,
		activeAttachmentId,
		router,
		project,
		workspaceName,
		isCurrentLayoutInstance,
	]);

	const navigateToNextTab = useCallback(() => {
		if (!isCurrentLayoutInstance()) return;
		const liveSessions = sessions.filter(
			(s) => !deletingIds.has(s.attachment_id),
		);
		if (liveSessions.length === 0) return;
		const activeIndex = liveSessions.findIndex(
			(s) => s.type === activeKind && s.attachment_id === activeAttachmentId,
		);
		if (activeIndex === -1) return;
		const nextIndex =
			activeIndex === liveSessions.length - 1 ? 0 : activeIndex + 1;
		const next = liveSessions[nextIndex];
		router.push(
			cloudSessionHref({
				project,
				workspace: workspaceName,
				kind: next.type,
				attachmentId: next.attachment_id,
			}),
		);
	}, [
		sessions,
		deletingIds,
		activeKind,
		activeAttachmentId,
		router,
		project,
		workspaceName,
		isCurrentLayoutInstance,
	]);

	useEffect(() => {
		if (isTauri()) {
			return listenShortcutEvent<void>(shortcutEvents.newTab, () => {
				setNewTabOpen(true);
			});
		}

		const handler = (e: KeyboardEvent) => {
			if (e.metaKey && e.key === "t") {
				e.preventDefault();
				setNewTabOpen(true);
			}
			if (e.metaKey && e.shiftKey && e.code === "BracketLeft") {
				e.preventDefault();
				navigateToPreviousTab();
			}
			if (e.metaKey && e.shiftKey && e.code === "BracketRight") {
				e.preventDefault();
				navigateToNextTab();
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [navigateToPreviousTab, navigateToNextTab]);

	useEffect(() => {
		return listenShortcutEvent<void>(shortcutEvents.closeTab, () => {
			closeActiveTab();
		});
	}, [closeActiveTab]);

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		return listenShortcutEvent<void>(shortcutEvents.previousTab, () => {
			navigateToPreviousTab();
		});
	}, [navigateToPreviousTab]);

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		return listenShortcutEvent<void>(shortcutEvents.nextTab, () => {
			navigateToNextTab();
		});
	}, [navigateToNextTab]);

	const TAB_OPTIONS = useMemo(
		() => [
			{
				label: "Terminal",
				icon: <Terminal size={12} />,
				action: () => createTerminal.mutate(),
				pending: createTerminal.isPending,
			},
			{
				label: "Codex",
				icon: <CodexIcon height={12} />,
				action: () => createAssistant.mutate("codex"),
				pending:
					createAssistant.isPending && createAssistant.variables === "codex",
			},
			{
				label: "Claude",
				icon: <ClaudeIcon height={12} />,
				action: () => createAssistant.mutate("claude"),
				pending:
					createAssistant.isPending && createAssistant.variables === "claude",
			},
			{
				label: "Browser",
				icon: <Globe size={12} />,
				action: () => createBrowser.mutate(),
				pending: createBrowser.isPending,
			},
		],
		[createTerminal, createAssistant, createBrowser],
	);

	useEffect(() => {
		if (!newTabOpen) return;
		const handler = (e: KeyboardEvent) => {
			const num = Number.parseInt(e.key, 10);
			if (num >= 1 && num <= TAB_OPTIONS.length) {
				e.preventDefault();
				TAB_OPTIONS[num - 1].action();
			}
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [newTabOpen, TAB_OPTIONS]);

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
			{sessions.length > 0 && (
				<div className="w-full bg-bg shrink-0 flex items-end overflow-x-auto">
					{sessions.map((session) => (
						<WorkspaceTab
							key={session.attachment_id}
							session={session}
							isActive={
								activeKind === session.type &&
								activeAttachmentId === session.attachment_id
							}
							isDeleting={deletingIds.has(session.attachment_id)}
							onClose={() => closeTab(session)}
							workspaceName={workspaceName}
							project={project}
						/>
					))}
					<Tooltip>
						<TooltipTrigger asChild>
							<button
								type="button"
								onClick={() => setNewTabOpen(true)}
								className="h-9 flex items-center px-2.5 border-b border-border-light text-text-muted hover:text-text-bright transition-colors"
							>
								<Plus size={12} />
							</button>
						</TooltipTrigger>
						<TooltipContent side="right">
							<span className="flex items-center gap-1.5">
								New tab
								<span className="flex items-center gap-0.5">
									<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
										⌘
									</kbd>
									<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-[9px] text-text">
										T
									</kbd>
								</span>
							</span>
						</TooltipContent>
					</Tooltip>
					<div className="flex-1 h-9 border-b border-border-light" />
				</div>
			)}
			<Dialog open={newTabOpen} onOpenChange={setNewTabOpen}>
				<DialogContent className="max-w-xs p-0 gap-0">
					<DialogHeader className="p-4 pb-2">
						<DialogTitle>New Tab</DialogTitle>
					</DialogHeader>
					<div className="flex flex-col pt-1 pb-3">
						{TAB_OPTIONS.map((option, index) => (
							<button
								key={option.label}
								type="button"
								disabled={isPending}
								onClick={option.action}
								className="flex items-center gap-2.5 w-full px-4 py-2 text-xs text-text hover:bg-btn-hover hover:text-text-bright transition-colors disabled:opacity-50"
							>
								{option.icon}
								<span className="truncate flex-1 text-left">
									{option.label}
								</span>
								<span className="shrink-0 w-5 h-5 inline-flex items-center justify-center">
									{option.pending ? (
										<Loader className="text-text-muted" />
									) : (
										<kbd className="text-[10px] text-text-placeholder border border-border-light rounded px-1.5 py-0.5">
											{index + 1}
										</kbd>
									)}
								</span>
							</button>
						))}
					</div>
				</DialogContent>
			</Dialog>
			{children}
		</>
	);
}

function WorkspaceTab({
	session,
	isActive,
	isDeleting,
	onClose,
	workspaceName,
	project,
}: {
	session: WorkspaceSession;
	isActive: boolean;
	isDeleting: boolean;
	onClose: () => void;
	workspaceName: string;
	project: string;
}) {
	const router = useRouter();

	const { icon, label } =
		session.type === "browser"
			? browserTabPresentation(session)
			: terminalTabPresentation(session.name);

	return (
		<div
			role="tab"
			aria-selected={isActive}
			tabIndex={0}
			onClick={() => {
				if (isDeleting) return;
				router.push(
					cloudSessionHref({
						project,
						workspace: workspaceName,
						kind: session.type,
						attachmentId: session.attachment_id,
					}),
				);
			}}
			onKeyDown={(event) => {
				if (event.key !== "Enter" && event.key !== " ") {
					return;
				}
				if (isDeleting) return;
				event.preventDefault();
				router.push(
					cloudSessionHref({
						project,
						workspace: workspaceName,
						kind: session.type,
						attachmentId: session.attachment_id,
					}),
				);
			}}
			className={`group/tab h-9 flex items-center gap-1.5 pl-3.5 pr-2.5 text-[11px] shrink-0 transition-colors border-r border-b cursor-pointer ${
				isActive
					? "bg-surface text-text-bright border-r-border-light border-b-surface"
					: "text-text border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			{icon}
			<span className="max-w-36 truncate">{label}</span>
			{isDeleting ? (
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
						onClick={(event) => {
							event.stopPropagation();
							onClose();
						}}
						className="hidden group-hover/unread:block p-0.5 rounded transition-colors hover:bg-border-light text-text-muted hover:text-text-bright"
					>
						<X size={10} />
					</button>
				</span>
			) : (
				<button
					type="button"
					onClick={(event) => {
						event.stopPropagation();
						onClose();
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
