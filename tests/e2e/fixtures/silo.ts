import path from "node:path";
import { test as base, expect, type Page } from "@playwright/test";
import {
	disconnectFromDriverSession,
	launchDriverSession,
	stopLaunchedSession,
} from "../../../driver/runtime";
import type { DriverSessionRecord } from "../../../driver/types";

type SiloApp = {
	artifactsDir: string;
	page: Page;
	session: DriverSessionRecord;
	stateDir: string;
};

export const test = base.extend<{ appPage: Page }, { siloApp: SiloApp }>({
	siloApp: [
		async ({ browserName }, use, workerInfo) => {
			void browserName;
			const timestamp = new Date().toISOString().replace(/:/g, "-");
			const artifactsDir = path.join(
				process.cwd(),
				"test-results",
				"e2e",
				`worker-${workerInfo.workerIndex}-${timestamp}`,
			);
			const launched = await launchDriverSession({ artifactsDir });

			try {
				await use({
					artifactsDir,
					page: launched.page,
					session: launched.session,
					stateDir: launched.session.stateDir,
				});
			} finally {
				await disconnectFromDriverSession(launched);
				await stopLaunchedSession(launched.session);
			}
		},
		{ scope: "worker" },
	],
	appPage: async ({ siloApp }, use) => {
		await use(siloApp.page);
	},
});

export { expect };
