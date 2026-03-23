import { getIdentifier } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { check } from "@tauri-apps/plugin-updater";
import { useEffect, useEffectEvent, useRef, useState } from "react";
import { toast } from "@/shared/ui/toaster";

const DEV_IDENTIFIER_SUFFIX = ".dev";
const UPDATE_CHECK_INTERVAL_MS = 15 * 60_000;

export function AppUpdater() {
	const [updaterEnabled, setUpdaterEnabled] = useState(false);
	const hasInstalledUpdateRef = useRef(false);
	const updateCheckInFlightRef = useRef(false);

	useEffect(() => {
		let cancelled = false;

		void (async () => {
			if (!isTauri()) {
				return;
			}

			try {
				const identifier = await getIdentifier();
				if (!cancelled) {
					setUpdaterEnabled(!identifier.endsWith(DEV_IDENTIFIER_SUFFIX));
				}
			} catch (error) {
				console.error("failed to resolve app identifier for updater", error);
			}
		})();

		return () => {
			cancelled = true;
		};
	}, []);

	const checkForUpdates = useEffectEvent(async () => {
		if (!updaterEnabled || hasInstalledUpdateRef.current) {
			return;
		}

		if (updateCheckInFlightRef.current) {
			return;
		}

		updateCheckInFlightRef.current = true;
		try {
			const update = await check();
			if (!update) {
				return;
			}

			await update.downloadAndInstall();
			hasInstalledUpdateRef.current = true;
			toast({
				title: "Update ready",
				description: "Restart to apply the latest version.",
				duration: Infinity,
				action: (
					<button
						type="button"
						className="ml-auto shrink-0 rounded-md bg-btn px-2.5 py-1 text-sm font-medium text-text-bright transition-colors hover:bg-btn-hover"
						onClick={() => invoke("system_restart_app")}
					>
						Restart
					</button>
				),
			});
		} catch (error) {
			console.error("failed to check for updates", error);
		} finally {
			updateCheckInFlightRef.current = false;
		}
	});

	useEffect(() => {
		if (!updaterEnabled) {
			return;
		}

		void checkForUpdates();
		const intervalId = window.setInterval(() => {
			void checkForUpdates();
		}, UPDATE_CHECK_INTERVAL_MS);

		return () => {
			window.clearInterval(intervalId);
		};
	}, [checkForUpdates, updaterEnabled]);

	return null;
}
