import { describe, expect, test } from "bun:test";
import {
	isPageForeground,
	resolveForegroundPollInterval,
} from "./page-foreground";

describe("isPageForeground", () => {
	test("requires a visible and focused document", () => {
		expect(
			isPageForeground({
				hasFocus: () => true,
				visibilityState: "visible",
			}),
		).toBe(true);
		expect(
			isPageForeground({
				hasFocus: () => false,
				visibilityState: "visible",
			}),
		).toBe(false);
		expect(
			isPageForeground({
				hasFocus: () => true,
				visibilityState: "hidden",
			}),
		).toBe(false);
	});

	test("treats a missing document as foreground", () => {
		expect(isPageForeground(null)).toBe(true);
		expect(isPageForeground(undefined)).toBe(true);
	});
});

describe("resolveForegroundPollInterval", () => {
	test("returns the active interval when foregrounded and active", () => {
		expect(
			resolveForegroundPollInterval({
				activeMs: 2000,
				isForeground: true,
			}),
		).toBe(2000);
	});

	test("uses the inactive interval when foregrounded but inactive", () => {
		expect(
			resolveForegroundPollInterval({
				active: false,
				activeMs: 2000,
				inactiveMs: 15000,
				isForeground: true,
			}),
		).toBe(15000);
	});

	test("pauses or slows polling when backgrounded", () => {
		expect(
			resolveForegroundPollInterval({
				activeMs: 2000,
				hiddenMs: false,
				isForeground: false,
			}),
		).toBe(false);
		expect(
			resolveForegroundPollInterval({
				activeMs: 2000,
				hiddenMs: 30000,
				isForeground: false,
			}),
		).toBe(30000);
	});

	test("disables polling when the query is disabled", () => {
		expect(
			resolveForegroundPollInterval({
				activeMs: 2000,
				enabled: false,
				isForeground: true,
			}),
		).toBe(false);
	});
});
