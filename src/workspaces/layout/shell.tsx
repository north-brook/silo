import {
	useMutation,
	useMutationState,
	useQueryClient,
} from "@tanstack/react-query";
import { Globe, Plus, Terminal, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Outlet, useLocation, useMatch, useNavigate } from "react-router-dom";
import type { TemplateOperation } from "@/projects/api";
import { domFocusSnapshot } from "@/shared/lib/focus-debug";
import { invoke } from "@/shared/lib/invoke";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "@/shared/ui/dialog";
import { ClaudeIcon } from "@/shared/ui/icons/claude";
import { CodexIcon } from "@/shared/ui/icons/codex";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
	isTemplateWorkspace,
	type Workspace,
	type WorkspaceSession,
} from "@/workspaces/api";
import {
	type DisplayWorkspaceSession,
	FileSessionsProvider,
	useFileSessions,
} from "@/workspaces/files/context";
import { FileIcon } from "@/workspaces/files/icons";
import { GitSidebarProvider } from "@/workspaces/git/context";
import { GitSidebar } from "@/workspaces/git/sidebar";
import { useSessionHosts } from "@/workspaces/hosts/provider";
import { TopBar } from "@/workspaces/layout/top-bar";
import { PromptDraftProvider } from "@/workspaces/prompt/draft";
import { useWorkspaceRouteParams } from "@/workspaces/routes/params";
import {
	type SessionRouteState,
	workspaceHref,
	workspaceSessionHref,
} from "@/workspaces/routes/paths";
import { TemplateOperationScreen } from "@/workspaces/routes/transition-screens";
import {
	RouteWorkspaceStateProvider,
	useCloudSessions,
	useTemplateState,
	useWorkspaceProject,
	useWorkspaceReady,
	useWorkspaceSessions,
	useWorkspaceState,
} from "@/workspaces/state";
import { removeWorkspaceSessionFromWorkspace } from "@/workspaces/state-events";
import { assistantTerminalModel } from "@/workspaces/terminal/session";

export function WorkspaceShell() {
	const { project, workspaceName } = useWorkspaceRouteParams();

	return (
		<RouteWorkspaceStateProvider
			project={project}
			workspaceName={workspaceName}
		>
			<PromptDraftProvider>
				<FileSessionsProvider>
					<GitSidebarProvider>
						<WorkspaceShellInner />
					</GitSidebarProvider>
				</FileSessionsProvider>
			</PromptDraftProvider>
		</RouteWorkspaceStateProvider>
	);
}

function terminalTabPresentation(name: string) {
	const trimmed = name.trim();
	const assistant = assistantTerminalModel(name);
	if (assistant === "claude") {
		return {
			icon: <ClaudeIcon height={12} />,
			label: "claude",
		};
	}
	if (assistant === "codex") {
		return {
			icon: <CodexIcon height={12} />,
			label: "codex",
		};
	}
	if (trimmed && trimmed !== "shell") {
		return {
			icon: <Terminal size={12} className="text-purple-400" />,
			label: trimmed,
		};
	}
	return {
		icon: <Terminal size={12} />,
		label: "shell",
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

function fileTabPresentation(session: WorkspaceSession) {
	return {
		icon: <FileIcon path={session.path ?? session.name} size={12} />,
		label: session.name.trim() || session.path?.trim() || "file",
	};
}

function templateLifecycleOperationFromWorkspace(
	workspace: Workspace | null,
): TemplateOperation | null {
	if (
		!workspace ||
		!isTemplateWorkspace(workspace) ||
		!workspace.template_operation
	) {
		return null;
	}

	const operation = workspace.template_operation;
	if (operation.kind !== "save" && operation.kind !== "delete") {
		return null;
	}

	return {
		project: workspace.project ?? "",
		workspace_name: workspace.name,
		kind: operation.kind as TemplateOperation["kind"],
		status: operation.phase === "failed" ? "failed" : "running",
		phase: operation.phase,
		detail: operation.detail ?? null,
		last_error: operation.last_error ?? null,
		snapshot_name: operation.snapshot_name ?? null,
		updated_at: operation.updated_at ?? workspace.created_at,
	};
}

function isLocalOnlyFileSession(
	session: DisplayWorkspaceSession,
	workspaceSessions: WorkspaceSession[],
) {
	return (
		session.type === "file" &&
		!session.persistentAttachmentId &&
		!workspaceSessions.some(
			(candidate) =>
				candidate.type === "file" &&
				candidate.attachment_id === session.attachment_id,
		)
	);
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

function WorkspaceShellInner() {
	const location = useLocation();
	const pathname = location.pathname;
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const savingRoutedRef = useRef(false);
	const browserMatch = useMatch(
		"/projects/:project/workspaces/:workspace/browser/:attachmentId",
	);
	const fileMatch = useMatch(
		"/projects/:project/workspaces/:workspace/file/:attachmentId",
	);
	const terminalMatch = useMatch(
		"/projects/:project/workspaces/:workspace/terminal/:attachmentId",
	);
	const { ensureWorkspaceSessions, removeSession } = useSessionHosts();
	const { clearSession, getDisplaySessions } = useFileSessions();
	const { invalidateWorkspace, isMissing, workspace, workspaceName } =
		useWorkspaceState();
	const project = useWorkspaceProject();
	const isWorkspaceReady = useWorkspaceReady();
	const templateState = useTemplateState(
		(workspace ? isTemplateWorkspace(workspace) : false) || isMissing
			? project
			: null,
	);
	const workspaceTemplateOperation =
		templateLifecycleOperationFromWorkspace(workspace);
	const templateStateOperation =
		templateState.data?.operation &&
		(templateState.data.operation.kind === "save" ||
			templateState.data.operation.kind === "delete")
			? templateState.data.operation
			: null;
	const templateLifecycleOperation = isMissing
		? (templateStateOperation ?? workspaceTemplateOperation)
		: (workspaceTemplateOperation ?? templateStateOperation);
	const workspaceSessions = useWorkspaceSessions();
	const sessions = useMemo(
		() => getDisplaySessions(workspaceSessions),
		[getDisplaySessions, workspaceSessions],
	);
	const cloudSessions = useCloudSessions();
	const [newTabOpen, setNewTabOpen] = useState(false);
	const activeKind = browserMatch
		? "browser"
		: fileMatch
			? "file"
			: terminalMatch
				? "terminal"
				: null;
	const activeAttachmentId =
		browserMatch?.params.attachmentId ??
		fileMatch?.params.attachmentId ??
		terminalMatch?.params.attachmentId ??
		null;
	const currentWorkspaceHref = useMemo(
		() => workspaceHref({ project, workspace: workspaceName }),
		[project, workspaceName],
	);

	useEffect(() => {
		if (
			!templateState.data?.operation ||
			templateState.data.operation.status !== "completed" ||
			templateState.data.workspace_present !== false ||
			savingRoutedRef.current
		) {
			return;
		}

		savingRoutedRef.current = true;
		const timer = window.setTimeout(
			() => navigate("/", { replace: true }),
			1500,
		);
		return () => {
			window.clearTimeout(timer);
		};
	}, [navigate, templateState.data]);

	const isCurrentLayoutInstance = useCallback(() => {
		if (typeof window === "undefined") {
			return true;
		}
		return window.location.pathname === pathname;
	}, [pathname]);

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
			invalidateWorkspace();
			navigate(
				workspaceSessionHref({
					project,
					workspace: workspaceName,
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

	const createAssistant = useMutation({
		mutationFn: (model: "codex" | "claude") =>
			invoke<{ attachment_id: string }>("terminal_create_assistant", {
				workspace: workspaceName,
				model,
			}),
		onSuccess: (result) => {
			setNewTabOpen(false);
			invalidateWorkspace();
			navigate(
				workspaceSessionHref({
					project,
					workspace: workspaceName,
					kind: "terminal",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
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
			invalidateWorkspace();
			navigate(
				workspaceSessionHref({
					project,
					workspace: workspaceName,
					kind: "browser",
					attachmentId: result.attachment_id,
				}),
				{ state: { fresh: true } satisfies SessionRouteState },
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
		mutationFn: (session: DisplayWorkspaceSession) =>
			session.type === "browser"
				? invoke("browser_kill_tab", {
						workspace: workspaceName,
						attachmentId: session.attachment_id,
					})
				: session.type === "file"
					? invoke("files_close_session", {
							workspace: workspaceName,
							attachmentId:
								session.persistentAttachmentId ?? session.attachment_id,
						})
					: invoke("terminal_kill_terminal", {
							workspace: workspaceName,
							attachmentId: session.attachment_id,
						}),
		onMutate: (session) => {
			const previousWorkspace = queryClient.getQueryData<Workspace | null>([
				"workspaces_get_workspace",
				workspaceName,
			]);
			const attachmentId =
				session.type === "file"
					? (session.persistentAttachmentId ?? session.attachment_id)
					: session.attachment_id;
			queryClient.setQueryData<Workspace | null>(
				["workspaces_get_workspace", workspaceName],
				(current) =>
					removeWorkspaceSessionFromWorkspace(current, {
						kind: session.type,
						attachmentId,
					}) ?? null,
			);
			return { previousWorkspace };
		},
		onSuccess: (_, session) => {
			invalidateWorkspace();
			removeSession(
				workspaceName,
				session.type,
				session.type === "file"
					? (session.persistentAttachmentId ?? session.attachment_id)
					: session.attachment_id,
			);
			clearSession(session.attachment_id);
		},
		onError: (error, session, context) => {
			queryClient.setQueryData<Workspace | null>(
				["workspaces_get_workspace", workspaceName],
				context?.previousWorkspace ?? null,
			);
			toast({
				variant: "error",
				title:
					session.type === "browser"
						? "Failed to close browser"
						: session.type === "file"
							? "Failed to close file"
							: "Failed to close terminal",
				description: error.message,
			});
		},
		onSettled: () => {
			invalidateWorkspace();
		},
	});

	const pendingKills = useMutationState({
		filters: {
			mutationKey: ["kill-session", workspaceName],
			status: "pending",
		},
		select: (mutation) =>
			(mutation.state.variables as DisplayWorkspaceSession | undefined)
				?.attachment_id,
	});
	const deletingIds = useMemo(
		() => new Set(pendingKills.filter((id): id is string => !!id)),
		[pendingKills],
	);

	const closeTab = useCallback(
		(session: DisplayWorkspaceSession) => {
			if (deletingIds.has(session.attachment_id)) return;

			const isLocalOnly = isLocalOnlyFileSession(session, workspaceSessions);
			const isActive =
				activeKind === session.type &&
				activeAttachmentId === session.attachment_id;
			const navigateTo = isActive
				? (() => {
						const idx = sessions.findIndex(
							(s) => s.attachment_id === session.attachment_id,
						);
						const preview = new Set(deletingIds);
						preview.add(session.attachment_id);
						const neighbor = findLiveNeighbor(sessions, idx, preview);
						return neighbor
							? workspaceSessionHref({
									project,
									workspace: workspaceName,
									kind: neighbor.type,
									attachmentId: neighbor.attachment_id,
								})
							: currentWorkspaceHref;
					})()
				: null;

			if (!session.preview && !isLocalOnly) {
				killSession.mutate(session);
			}

			if (isActive) {
				navigate(navigateTo ?? currentWorkspaceHref, { replace: true });
			}

			if (session.preview || isLocalOnly) {
				clearSession(session.attachment_id);
				return;
			}
		},
		[
			deletingIds,
			activeKind,
			activeAttachmentId,
			sessions,
			navigate,
			project,
			workspaceName,
			currentWorkspaceHref,
			clearSession,
			killSession,
			workspaceSessions,
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
		navigate(
			workspaceSessionHref({
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
		navigate,
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
		navigate(
			workspaceSessionHref({
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
		navigate,
		project,
		workspaceName,
		isCurrentLayoutInstance,
	]);

	useShortcut<void>({
		event: shortcutEvents.newTab,
		onTrigger: () => {
			console.info("new tab dialog requested", {
				source: "shortcut-event",
				activeKind,
				activeAttachmentId,
				...domFocusSnapshot(),
			});
			setNewTabOpen(true);
		},
		onKeyDown: (e) => {
			if (e.metaKey && e.key === "t") {
				e.preventDefault();
				console.info("new tab dialog requested", {
					source: "keydown",
					activeKind,
					activeAttachmentId,
					...domFocusSnapshot(),
				});
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
		},
	});
	useShortcut<void>({
		event: shortcutEvents.newTerminalTab,
		onTrigger: () => {
			createTerminal.mutate();
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey || e.altKey || e.ctrlKey) return;
			if (e.key.toLowerCase() === "t") {
				e.preventDefault();
				createTerminal.mutate();
			}
		},
	});
	useShortcut<void>({
		event: shortcutEvents.newBrowserTab,
		onTrigger: () => {
			createBrowser.mutate();
		},
		onKeyDown: (e) => {
			if (!e.metaKey || !e.shiftKey || e.altKey || e.ctrlKey) return;
			if (e.key.toLowerCase() === "b") {
				e.preventDefault();
				createBrowser.mutate();
			}
		},
	});
	useShortcut<void>({
		event: shortcutEvents.closeTab,
		onTrigger: () => {
			closeActiveTab();
		},
	});
	useShortcut<void>({
		event: shortcutEvents.previousTab,
		onTrigger: () => {
			navigateToPreviousTab();
		},
	});
	useShortcut<void>({
		event: shortcutEvents.nextTab,
		onTrigger: () => {
			navigateToNextTab();
		},
	});

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

		console.info("new tab dialog opened", {
			activeKind,
			activeAttachmentId,
			...domFocusSnapshot(),
		});

		const handler = (e: KeyboardEvent) => {
			if (e.key === "Escape" || /^[1-9]$/.test(e.key)) {
				console.info("new tab dialog keydown", {
					key: e.key,
					metaKey: e.metaKey,
					ctrlKey: e.ctrlKey,
					altKey: e.altKey,
					shiftKey: e.shiftKey,
					...domFocusSnapshot(),
				});
			}
			const num = Number.parseInt(e.key, 10);
			if (num >= 1 && num <= TAB_OPTIONS.length) {
				e.preventDefault();
				TAB_OPTIONS[num - 1].action();
			}
		};
		window.addEventListener("keydown", handler);
		return () => {
			console.info("new tab dialog closed", {
				activeKind,
				activeAttachmentId,
				...domFocusSnapshot(),
			});
			window.removeEventListener("keydown", handler);
		};
	}, [activeAttachmentId, activeKind, newTabOpen, TAB_OPTIONS]);

	return (
		<>
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
								className="flex items-center gap-2.5 w-full px-4 py-2 text-sm text-text hover:bg-btn-hover hover:text-text-bright transition-colors disabled:opacity-50"
							>
								{option.icon}
								<span className="truncate flex-1 text-left">
									{option.label}
								</span>
								<span className="shrink-0 w-5 h-5 inline-flex items-center justify-center">
									{option.pending ? (
										<Loader className="text-text-muted" />
									) : (
										<kbd className="text-sm text-text-placeholder border border-border-light rounded px-1.5 py-0.5">
											{index + 1}
										</kbd>
									)}
								</span>
							</button>
						))}
					</div>
				</DialogContent>
			</Dialog>
			<div className="flex-1 min-h-0 flex">
				<div className="flex-1 min-w-0 overflow-hidden flex flex-col">
					{workspace ? (
						<TopBar workspace={workspace} />
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
											<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text">
												⌘
											</kbd>
											<kbd className="inline-flex items-center justify-center w-4 h-4 rounded border border-border-light text-xs text-text">
												T
											</kbd>
										</span>
									</span>
								</TooltipContent>
							</Tooltip>
							<div className="flex-1 h-9 border-b border-border-light" />
						</div>
					)}
					<div className="flex-1 min-h-0 overflow-hidden flex flex-col">
						{templateLifecycleOperation ? (
							<TemplateOperationScreen operation={templateLifecycleOperation} />
						) : (
							<Outlet />
						)}
					</div>
				</div>
				<GitSidebar />
			</div>
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
	session: DisplayWorkspaceSession;
	isActive: boolean;
	isDeleting: boolean;
	onClose: () => void;
	workspaceName: string;
	project: string;
}) {
	const navigate = useNavigate();
	const { getSessionState } = useFileSessions();
	const fileState = getSessionState(session.attachment_id);

	const { icon, label } =
		session.type === "browser"
			? browserTabPresentation(session)
			: session.type === "file"
				? fileTabPresentation(session)
				: terminalTabPresentation(session.name);

	return (
		<div
			role="tab"
			aria-selected={isActive}
			tabIndex={0}
			onClick={() => {
				if (isDeleting) return;
				navigate(
					workspaceSessionHref({
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
				navigate(
					workspaceSessionHref({
						project,
						workspace: workspaceName,
						kind: session.type,
						attachmentId: session.attachment_id,
					}),
				);
			}}
			className={`group/tab h-9 flex items-center gap-1.5 pl-3.5 pr-2.5 text-sm shrink-0 transition-colors border-r border-b cursor-pointer ${
				isActive
					? "bg-surface text-text-bright border-r-border-light border-b-surface"
					: "text-text border-r-border-light border-b-border-light hover:bg-btn-hover hover:text-text-bright"
			}`}
		>
			{icon}
			<span className={`max-w-36 truncate ${session.preview ? "italic" : ""}`}>
				{label}
			</span>
			{isDeleting ? (
				<span className="p-0.5">
					<Loader className="text-error" />
				</span>
			) : session.type === "file" && fileState.saving ? (
				<span className="p-0.5">
					<Loader className="text-accent" />
				</span>
			) : session.type === "file" && fileState.conflicted ? (
				<span className="shrink-0 w-2 h-2 rounded-full bg-red-400" />
			) : session.type === "file" && fileState.dirty ? (
				<span className="shrink-0 w-2 h-2 rounded-full bg-amber-300" />
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
