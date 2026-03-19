import type { Locator, Page } from "@playwright/test";
import type { ParsedSelector } from "./types";

const ROLE_PATTERN =
	/^role:([a-z0-9_-]+)(?:\[name=(?:"([^"]+)"|'([^']+)')\])?$/i;

export function parseSelector(selector: string): ParsedSelector {
	if (selector.startsWith("testid:")) {
		return { kind: "testid", value: selector.slice("testid:".length) };
	}

	if (selector.startsWith("text:")) {
		return { kind: "text", value: selector.slice("text:".length) };
	}

	if (selector.startsWith("label:")) {
		return { kind: "label", value: selector.slice("label:".length) };
	}

	if (selector.startsWith("css:")) {
		return { kind: "css", value: selector.slice("css:".length) };
	}

	const roleMatch = selector.match(ROLE_PATTERN);
	if (roleMatch) {
		const [, role, doubleQuotedName, singleQuotedName] = roleMatch;
		return {
			kind: "role",
			role,
			name: doubleQuotedName ?? singleQuotedName ?? undefined,
		};
	}

	return { kind: "css", value: selector };
}

export function resolveLocator(page: Page, selector: string): Locator {
	const parsed = parseSelector(selector);

	switch (parsed.kind) {
		case "testid":
			return page.getByTestId(parsed.value);
		case "text":
			return page.getByText(parsed.value);
		case "label":
			return page.getByLabel(parsed.value);
		case "role":
			return page.getByRole(parsed.role as Parameters<Page["getByRole"]>[0], {
				name: parsed.name,
			});
		case "css":
			return page.locator(parsed.value);
	}
}
