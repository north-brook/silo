import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const WINDOW_SHORTCUT_EVENT = "__silo_native_shortcut__";

type WindowShortcutDetail = {
	event: string;
	payload?: unknown;
};

export const shortcutEvents = {
	newWorkspace: "silo://new-workspace",
	openProject: "silo://open-project",
	newTab: "silo://new-tab",
	closeTab: "silo://close-tab",
	goBackBrowser: "silo://go-back-browser",
	goForwardBrowser: "silo://go-forward-browser",
	refreshBrowser: "silo://refresh-browser",
	toggleBrowserDevtools: "silo://toggle-browser-devtools",
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
	// Frontend shortcut consumers listen in one place and should not care whether
	// the source was the normal Tauri menu event or a CEF-specific fallback.
	// New shortcuts should usually be added in the native menu/backend first.
	let disposed = false;
	let unlisten: null | (() => void | Promise<void>) = null;
	const windowShortcutHandler = (windowEvent: Event) => {
		const detail = (windowEvent as CustomEvent<WindowShortcutDetail>).detail;
		if (!detail || detail.event !== event) {
			return;
		}
		handler(detail.payload as T);
	};

	if (typeof window !== "undefined") {
		window.addEventListener(
			WINDOW_SHORTCUT_EVENT,
			windowShortcutHandler as EventListener,
		);
	}

	const disposeListener = (nextUnlisten: null | (() => void | Promise<void>)) => {
		if (!nextUnlisten) {
			return;
		}

		void Promise.resolve(nextUnlisten()).catch(() => {
			// Tauri can race listener registration and component teardown on fast route changes.
		});
	};

	if (isTauri()) {
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
	}

	return () => {
		disposed = true;
		if (typeof window !== "undefined") {
			window.removeEventListener(
				WINDOW_SHORTCUT_EVENT,
				windowShortcutHandler as EventListener,
			);
		}
		disposeListener(unlisten);
	};
}
