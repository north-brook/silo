"use client";

import { isTauri } from "@tauri-apps/api/core";
import { useEffect, useEffectEvent } from "react";
import { listenShortcutEvent } from "./shortcuts";

interface UseShortcutOptions<T> {
	enabled?: boolean;
	event: string;
	onKeyDown?: (event: KeyboardEvent, trigger: (payload: T) => void) => void;
	onTrigger: (payload: T) => void;
}

export function useShortcut<T>({
	enabled = true,
	event,
	onKeyDown,
	onTrigger,
}: UseShortcutOptions<T>) {
	const handleTrigger = useEffectEvent((payload: T) => {
		onTrigger(payload);
	});
	const handleKeyDown = useEffectEvent((keyboardEvent: KeyboardEvent) => {
		if (!onKeyDown) {
			return;
		}

		onKeyDown(keyboardEvent, (payload: T) => {
			handleTrigger(payload);
		});
	});
	const hasBrowserFallback = onKeyDown !== undefined;

	useEffect(() => {
		if (!enabled) {
			return;
		}

		const disposeShortcut = listenShortcutEvent<T>(event, (payload) => {
			handleTrigger(payload);
		});
		if (!hasBrowserFallback || isTauri()) {
			return disposeShortcut;
		}

		const keydownHandler = (keyboardEvent: KeyboardEvent) => {
			handleKeyDown(keyboardEvent);
		};

		window.addEventListener("keydown", keydownHandler);
		return () => {
			window.removeEventListener("keydown", keydownHandler);
			disposeShortcut();
		};
	}, [enabled, event, hasBrowserFallback]);
}
