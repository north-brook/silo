import { describe, expect, test } from "bun:test";
import {
	type BrowserStateEventPayload,
	popupBrowserSessionHrefForEvent,
} from "./events";

function event(
	overrides: Partial<BrowserStateEventPayload> = {},
): BrowserStateEventPayload {
	return {
		workspace: "demo-silo",
		attachmentId: "browser-1",
		popupAttachmentId: null,
		...overrides,
	};
}

describe("popupBrowserSessionHrefForEvent", () => {
	test("returns the popup browser route for the current workspace", () => {
		expect(
			popupBrowserSessionHrefForEvent(
				event({
					popupAttachmentId: "browser-2",
				}),
				{
					project: "demo",
					workspaceName: "demo-silo",
				},
			),
		).toBe("/projects/demo/workspaces/demo-silo/browser/browser-2");
	});

	test("ignores non-popup browser state events", () => {
		expect(
			popupBrowserSessionHrefForEvent(event(), {
				project: "demo",
				workspaceName: "demo-silo",
			}),
		).toBeNull();
	});

	test("ignores popup events for other workspaces", () => {
		expect(
			popupBrowserSessionHrefForEvent(
				event({
					workspace: "other-silo",
					popupAttachmentId: "browser-2",
				}),
				{
					project: "demo",
					workspaceName: "demo-silo",
				},
			),
		).toBeNull();
	});
});
