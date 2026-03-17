"use client";

import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const shortcutEvents = {
	newWorkspace: "silo://new-workspace",
	openProject: "silo://open-project",
	newTab: "silo://new-tab",
	closeTab: "silo://close-tab",
	goBackBrowser: "silo://go-back-browser",
	goForwardBrowser: "silo://go-forward-browser",
	refreshBrowser: "silo://refresh-browser",
	previousTab: "silo://previous-tab",
	nextTab: "silo://next-tab",
	toggleProjectsBar: "silo://toggle-projects-bar",
	toggleGitBar: "silo://toggle-git-bar",
	gitCreateOrPushPr: "silo://git-create-or-push-pr",
	gitMergePr: "silo://git-merge-pr",
	jumpToWorkspace: "silo://jump-to-workspace",
} as const;

export function listenShortcutEvent<T>(
	event: string,
	handler: (payload: T) => void,
) {
	if (!isTauri()) {
		return () => {};
	}

	let disposed = false;
	let unlisten: null | (() => void) = null;

	void listen<T>(event, ({ payload }) => {
		handler(payload);
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
}
