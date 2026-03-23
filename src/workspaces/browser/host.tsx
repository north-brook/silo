import { useEffect, useEffectEvent, useRef, useState } from "react";
import { domFocusSnapshot } from "@/shared/lib/focus-debug";
import type { CloudSession } from "@/workspaces/hosts/model";
import { invoke } from "@/shared/lib/invoke";
import { useOverlayOpen } from "@/shared/ui/overlay-state";

type HostStatus = "idle" | "attaching" | "ready" | "error";

interface BrowserViewport {
	x: number;
	y: number;
	width: number;
	height: number;
}

const HIDDEN_VIEWPORT: BrowserViewport = {
	x: -20_000,
	y: 0,
	width: 1,
	height: 1,
};

export function BrowserSessionHost({
	session,
	target,
	workspaceActive,
	visible,
	onHostStateChange,
}: {
	session: CloudSession;
	target: HTMLElement | null;
	workspaceActive: boolean;
	visible: boolean;
	onHostStateChange: (state: {
		status: HostStatus;
		errorMessage?: string | null;
	}) => void;
}) {
	const onHostStateChangeRef = useRef(onHostStateChange);
	const lastViewportRef = useRef<string | null>(null);
	const [ensured, setEnsured] = useState(false);
	const ensuredRef = useRef(false);
	const overlayOpen = useOverlayOpen();
	const effectiveVisible = visible && !overlayOpen;

	useEffect(() => {
		onHostStateChangeRef.current = onHostStateChange;
	}, [onHostStateChange]);

	const concealBrowserTab = useEffectEvent(
		(
			cancelled: boolean,
			reason: "workspace-inactive" | "overlay-open",
		) => {
		if (!ensuredRef.current) {
			return;
		}

		console.info("browser host conceal requested", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			reason,
			overlayOpen,
			workspaceActive,
			visible,
			effectiveVisible,
			...domFocusSnapshot(),
		});

		const hiddenViewportKey = JSON.stringify({
			viewport: HIDDEN_VIEWPORT,
			visible: false,
		});
		if (lastViewportRef.current === hiddenViewportKey) {
			return;
		}
		const previousViewportKey = lastViewportRef.current;
		lastViewportRef.current = hiddenViewportKey;
		void invoke("browser_resize_tab", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			viewport: HIDDEN_VIEWPORT,
			visible: false,
		})
			.then(() => {
				if (cancelled) {
					return;
				}
				console.info("browser host concealed", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					reason,
					...domFocusSnapshot(),
				});
			})
			.catch((error: Error) => {
				if (cancelled) {
					return;
				}
				if (lastViewportRef.current === hiddenViewportKey) {
					lastViewportRef.current = previousViewportKey;
				}
				onHostStateChangeRef.current({
					status: "error",
					errorMessage: error.message,
				});
			});
		},
	);

	useEffect(() => {
		if (ensured || !target || !workspaceActive || !effectiveVisible) {
			return;
		}

		let cancelled = false;
		const rect = target.getBoundingClientRect();
		if (rect.width <= 0 || rect.height <= 0) {
			return;
		}

		const viewport: BrowserViewport = {
			x: rect.left,
			y: rect.top,
			width: rect.width,
			height: rect.height,
		};
		console.info("browser host mount start", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			viewport,
			effectiveVisible,
			...domFocusSnapshot(),
		});
		onHostStateChangeRef.current({ status: "attaching", errorMessage: null });
		void invoke("browser_mount_tab", {
			workspace: session.workspace,
			attachmentId: session.attachmentId,
			viewport,
			visible: effectiveVisible,
		})
			.then(() => {
				if (cancelled) {
					return;
				}
				ensuredRef.current = true;
				setEnsured(true);
				console.info("browser host mount ready", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					effectiveVisible,
					...domFocusSnapshot(),
				});
				onHostStateChangeRef.current({ status: "ready", errorMessage: null });
			})
			.catch((error: Error) => {
				if (cancelled) {
					return;
				}
				onHostStateChangeRef.current({
					status: "error",
					errorMessage: error.message,
				});
			});

		return () => {
			cancelled = true;
		};
	}, [
		ensured,
		session.attachmentId,
		session.workspace,
		target,
		effectiveVisible,
		workspaceActive,
	]);

	useEffect(() => {
		return () => {
			if (!ensuredRef.current) {
				return;
			}
			void invoke("browser_detach_tab", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			});
		};
	}, [session.attachmentId, session.workspace]);

	useEffect(() => {
		if (!ensured) {
			return;
		}

		let cancelled = false;
		let rafId: number | null = null;
		let unmountTimer: number | null = null;
		let resizeObserver: ResizeObserver | null = null;

		if (!workspaceActive || !target) {
			unmountTimer = window.setTimeout(() => {
				concealBrowserTab(cancelled, "workspace-inactive");
			}, 150);
			return () => {
				cancelled = true;
				if (unmountTimer !== null) {
					window.clearTimeout(unmountTimer);
				}
			};
		}

		if (!effectiveVisible) {
			concealBrowserTab(cancelled, "overlay-open");
			return () => {
				cancelled = true;
			};
		}

		const syncViewport = () => {
			const rect = target.getBoundingClientRect();
			if (rect.width <= 0 || rect.height <= 0) {
				return;
			}

			const viewport: BrowserViewport = {
				x: rect.left,
				y: rect.top,
				width: rect.width,
				height: rect.height,
			};
			const viewportKey = JSON.stringify({
				viewport,
				visible: effectiveVisible,
			});
			if (lastViewportRef.current === viewportKey) {
				return;
			}
			lastViewportRef.current = viewportKey;
			void invoke("browser_resize_tab", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
				viewport,
				visible: effectiveVisible,
			}).catch((error: Error) => {
				if (cancelled) {
					return;
				}
				onHostStateChangeRef.current({
					status: "error",
					errorMessage: error.message,
				});
			});
		};

		const requestSync = () => {
			if (rafId !== null) {
				window.cancelAnimationFrame(rafId);
			}
			rafId = window.requestAnimationFrame(() => {
				rafId = null;
				syncViewport();
			});
		};

		requestSync();
		window.addEventListener("resize", requestSync);
		resizeObserver = new ResizeObserver(() => requestSync());
		resizeObserver.observe(target);

		return () => {
			cancelled = true;
			if (rafId !== null) {
				window.cancelAnimationFrame(rafId);
			}
			if (unmountTimer !== null) {
				window.clearTimeout(unmountTimer);
			}
			window.removeEventListener("resize", requestSync);
			resizeObserver?.disconnect();
		};
	}, [
		concealBrowserTab,
		ensured,
		session.attachmentId,
		session.workspace,
		target,
		effectiveVisible,
		workspaceActive,
	]);

	return null;
}
