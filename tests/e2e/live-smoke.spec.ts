import { expect, test } from "./fixtures/silo";

test("launches with live Google Cloud and GitHub state", async ({
	appPage,
}) => {
	await expect(
		appPage.getByTestId("dashboard-action-open-project"),
	).toBeVisible();

	await expect
		.poll(() =>
			appPage
				.getByTestId("setup-status-gcloud")
				.getAttribute("data-status-label"),
		)
		.toBe("Google Cloud: connected");

	await expect
		.poll(() =>
			appPage
				.getByTestId("setup-status-github")
				.getAttribute("data-status-label"),
		)
		.toBe("GitHub: connected");
});
