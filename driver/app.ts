import type { Page } from "@playwright/test";
import type { AppServiceStatus, AppStatus } from "./types";
import { waitFor } from "./utils";

async function readStatusLabel(page: Page, testId: string) {
	return page.getByTestId(testId).getAttribute("data-status-label");
}

export async function readAppServiceStatuses(
	page: Page,
): Promise<AppServiceStatus> {
	const [gcloud, github, codex, claude] = await Promise.all([
		readStatusLabel(page, "setup-status-gcloud"),
		readStatusLabel(page, "setup-status-github"),
		readStatusLabel(page, "setup-status-codex"),
		readStatusLabel(page, "setup-status-claude"),
	]);

	return {
		claude,
		codex,
		gcloud,
		github,
	};
}

export async function waitForAppReady(page: Page, timeoutMs = 60_000) {
	await page.waitForLoadState("domcontentloaded");

	return waitFor(
		async () => {
			const openProjectVisible = await page
				.getByTestId("dashboard-action-open-project")
				.isVisible()
				.catch(() => false);
			if (!openProjectVisible) {
				return undefined;
			}

			const services = await readAppServiceStatuses(page);
			const allStatusesReady = Object.values(services).every(
				(value) => value !== null,
			);
			if (!allStatusesReady) {
				return undefined;
			}

			return {
				openProjectVisible,
				pageTitle: await page.title().catch(() => ""),
				pageUrl: page.url(),
				services,
			} satisfies AppStatus;
		},
		{ timeoutMs, description: "the Silo dashboard to become ready" },
	);
}

export async function readAppStatus(page: Page): Promise<AppStatus> {
	const services = await readAppServiceStatuses(page);
	const openProjectVisible = await page
		.getByTestId("dashboard-action-open-project")
		.isVisible()
		.catch(() => false);

	return {
		openProjectVisible,
		pageTitle: await page.title().catch(() => ""),
		pageUrl: page.url(),
		services,
	};
}
