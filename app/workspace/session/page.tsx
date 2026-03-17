"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ArrowLeft, ArrowRight, RotateCw } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo, useRef, useState } from "react";
import { CloudDeck } from "../../../components/cloud";
import { Loader } from "../../../components/loader";
import { toast } from "../../../components/toaster";
import {
	type CloudSession,
	normalizeWorkspaceSession,
} from "../../../lib/cloud";
import { invoke } from "../../../lib/invoke";
import { listenShortcutEvent, shortcutEvents } from "../../../lib/shortcuts";
import {
	isTemplateWorkspace,
	type Workspace,
	type WorkspaceSession,
	workspaceSessions,
} from "../../../lib/workspaces";

export default function WorkspaceSessionPage() {
	return (
		<Suspense>
			<WorkspaceSessionView />
		</Suspense>
	);
}

function WorkspaceSessionView() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const queryClient = useQueryClient();
	const workspace = searchParams.get("workspace") ?? "";
	const attachmentId = searchParams.get("attachment_id") ?? "";
	const kind = searchParams.get("kind") ?? "";
	const fresh = searchParams.get("fresh") === "1";
	const cleanParams = new URLSearchParams(searchParams.toString());
	cleanParams.delete("fresh");
	const cleanUrl = `/workspace/session?${cleanParams.toString()}`;

	useEffect(() => {
		if (!fresh) {
			return;
		}
		router.replace(cleanUrl);
	}, [cleanUrl, fresh, router]);

	useEffect(() => {
		if (!workspace) {
			return;
		}

		let disposed = false;
		let unlisten: (() => void) | null = null;
		void listen<{ workspace: string }>("browser://state", (event) => {
			if (disposed || event.payload.workspace !== workspace) {
				return;
			}
			queryClient.invalidateQueries({
				queryKey: ["workspaces_get_workspace", workspace],
			});
		}).then((nextUnlisten) => {
			if (disposed) {
				nextUnlisten();
				return;
			}
			unlisten = nextUnlisten;
		});

		return () => {
			disposed = true;
			unlisten?.();
		};
	}, [queryClient, workspace]);

	const workspaceQuery = useQuery({
		queryKey: ["workspaces_get_workspace", workspace],
		queryFn: () =>
			invoke<Workspace>(
				"workspaces_get_workspace",
				{ workspace },
				{
					log: "state_changes_only",
					key: `poll:workspaces_get_workspace:${workspace}`,
				},
			),
		enabled: !!workspace,
		refetchInterval: 2000,
	});

	const sessions = useMemo<WorkspaceSession[]>(
		() =>
			workspaceQuery.data && !isTemplateWorkspace(workspaceQuery.data)
				? workspaceSessions(workspaceQuery.data)
				: [],
		[workspaceQuery.data],
	);
	const hasLiveSession = useMemo(
		() =>
			sessions.some(
				(session) =>
					session.type === kind && session.attachment_id === attachmentId,
			),
		[attachmentId, kind, sessions],
	);
	const cloudSessions = useMemo(
		() =>
			sessions.map((session) => normalizeWorkspaceSession(workspace, session)),
		[sessions, workspace],
	);
	const activeSession = useMemo<CloudSession | null>(() => {
		if (!workspace || !attachmentId || !kind) {
			return null;
		}
		return (
			cloudSessions.find(
				(session) =>
					session.kind === kind && session.attachmentId === attachmentId,
			) ?? {
				workspace,
				kind,
				attachmentId,
				name: attachmentId,
				url: null,
				logicalUrl: null,
				resolvedUrl: null,
				title: null,
				faviconUrl: null,
				canGoBack: null,
				canGoForward: null,
				working: null,
				unread: null,
			}
		);
	}, [attachmentId, cloudSessions, kind, workspace]);

	useEffect(() => {
		if (!workspace || !kind || !attachmentId || !hasLiveSession) {
			return;
		}

		void invoke("workspaces_set_active_session", {
			workspace,
			kind,
			attachmentId,
		});
	}, [attachmentId, hasLiveSession, kind, workspace]);

	if (!workspace || !attachmentId || !kind || !activeSession) {
		return null;
	}

	if (kind === "terminal") {
		return (
			<CloudDeck
				workspace={workspace}
				activeSession={activeSession}
				skipInitialScrollback={fresh}
			/>
		);
	}

	if (kind !== "browser") {
		return (
			<div className="flex-1 min-h-0 bg-surface flex items-center justify-center p-6">
				<div className="text-[11px] text-text-muted">
					Unsupported session type: {kind}
				</div>
			</div>
		);
	}

	const hasUrl = !!activeSession.url;

	return (
		<div className="flex-1 min-h-0 bg-surface flex flex-col">
			<BrowserSessionHeader
				session={activeSession}
				autoFocusAddress={!hasUrl}
				onChanged={() =>
					queryClient.invalidateQueries({
						queryKey: ["workspaces_get_workspace", workspace],
					})
				}
			/>
			{hasUrl && (
				<CloudDeck
					workspace={workspace}
					activeSession={activeSession}
					skipInitialScrollback={false}
				/>
			)}
		</div>
	);
}

function BrowserSessionHeader({
	session,
	autoFocusAddress,
	onChanged,
}: {
	session: CloudSession;
	autoFocusAddress: boolean;
	onChanged: () => void;
}) {
	const inputRef = useRef<HTMLInputElement>(null);
	const [address, setAddress] = useState(session.url ?? "");

	useEffect(() => {
		setAddress(session.url ?? "");
	}, [session.url]);

	useEffect(() => {
		if (autoFocusAddress) {
			inputRef.current?.focus();
		}
	}, [autoFocusAddress]);

	const navigate = useMutation({
		mutationFn: (url: string) =>
			invoke("browser_go_to", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
				url,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to navigate",
				description: error.message,
			});
		},
	});

	const goBack = useMutation({
		mutationFn: () =>
			invoke("browser_go_back", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to go back",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		return listenShortcutEvent<void>(shortcutEvents.goBackBrowser, () => {
			if (session.canGoBack !== false && !goBack.isPending) {
				goBack.mutate();
			}
		});
	}, [goBack, session.canGoBack]);

	const goForward = useMutation({
		mutationFn: () =>
			invoke("browser_go_forward", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to go forward",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		return listenShortcutEvent<void>(shortcutEvents.goForwardBrowser, () => {
			if (session.canGoForward !== false && !goForward.isPending) {
				goForward.mutate();
			}
		});
	}, [goForward, session.canGoForward]);

	const refresh = useMutation({
		mutationFn: () =>
			invoke("browser_refresh_page", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to refresh",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		if (!isTauri()) {
			return;
		}

		return listenShortcutEvent<void>(shortcutEvents.refreshBrowser, () => {
			if (!refresh.isPending) {
				refresh.mutate();
			}
		});
	}, [refresh]);

	const busy =
		navigate.isPending ||
		goBack.isPending ||
		goForward.isPending ||
		refresh.isPending;

	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				navigate.mutate(address);
			}}
			className="h-9 shrink-0 bg-surface px-1.5 flex items-center gap-0.5"
		>
			<button
				type="button"
				disabled={busy || session.canGoBack === false}
				onClick={() => goBack.mutate()}
				aria-label="Back"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				<ArrowLeft size={12} />
			</button>
			<button
				type="button"
				disabled={busy || session.canGoForward === false}
				onClick={() => goForward.mutate()}
				aria-label="Forward"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				<ArrowRight size={12} />
			</button>
			<button
				type="button"
				disabled={busy}
				onClick={() => refresh.mutate()}
				aria-label="Refresh"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				{busy ? <Loader className="text-text-muted" /> : <RotateCw size={12} />}
			</button>
			<input
				ref={inputRef}
				value={address}
				onChange={(event) => setAddress(event.target.value)}
				placeholder="Enter URL"
				spellCheck={false}
				autoCorrect="off"
				autoCapitalize="off"
				className="flex-1 min-w-0 h-7 rounded-md bg-bg px-2.5 text-[12px] text-text-bright outline-none border border-border-light focus:border-text-muted transition-colors"
			/>
		</form>
	);
}
