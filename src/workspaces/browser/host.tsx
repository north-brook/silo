import { useEffect, useRef, useState } from "react";
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

	useEffect(() => {
		if (ensured || !target || !workspaceActive) {
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
			void invoke("browser_unmount_tab", {
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
				void invoke("browser_unmount_tab", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
				}).catch((error: Error) => {
					if (cancelled) {
						return;
					}
					onHostStateChangeRef.current({
						status: "error",
						errorMessage: error.message,
					});
				});
			}, 150);
			return () => {
				cancelled = true;
				if (unmountTimer !== null) {
					window.clearTimeout(unmountTimer);
				}
			};
		}

		if (!effectiveVisible) {
			const viewport: BrowserViewport = {
				x: -20_000,
				y: 0,
				width: 1,
				height: 1,
			};
			const viewportKey = JSON.stringify({ viewport, visible: false });
			if (lastViewportRef.current !== viewportKey) {
				lastViewportRef.current = viewportKey;
				void invoke("browser_resize_tab", {
					workspace: session.workspace,
					attachmentId: session.attachmentId,
					viewport,
					visible: false,
				}).catch((error: Error) => {
					if (cancelled) {
						return;
					}
					onHostStateChangeRef.current({
						status: "error",
						errorMessage: error.message,
					});
				});
			}
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
		ensured,
		session.attachmentId,
		session.workspace,
		target,
		effectiveVisible,
		workspaceActive,
	]);

	return null;
}
