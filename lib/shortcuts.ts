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
	openGitDiff: "silo://open-git-diff",
	openGitChecks: "silo://open-git-checks",
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
	let unlisten: null | (() => void | Promise<void>) = null;

	const disposeListener = (nextUnlisten: null | (() => void | Promise<void>)) => {
		if (!nextUnlisten) {
			return;
		}

		void Promise.resolve(nextUnlisten()).catch(() => {
			// Tauri can race listener registration and component teardown on fast route changes.
		});
	};

	void listen<T>(event, ({ payload }) => {
		handler(payload);
	})
		.then((nextUnlisten) => {
			if (disposed) {
				disposeListener(nextUnlisten);
				return;
			}
			unlisten = nextUnlisten;
		})
		.catch(() => {});

	return () => {
		disposed = true;
		disposeListener(unlisten);
	};
}
