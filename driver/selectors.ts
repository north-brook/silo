import type { Locator, Page } from "@playwright/test";
import type { ParsedSelector } from "./types";

const ROLE_PATTERN =
	/^role:([a-z0-9_-]+)(?:\[name=(?:"([^"]+)"|'([^']+)')\])?$/i;
const EXPLICIT_PREFIX_PATTERN = /^([a-z0-9_-]+):/i;
const KNOWN_PREFIXES = ["testid", "text", "label", "css", "role"] as const;

export function parseSelector(selector: string): ParsedSelector {
	if (selector.length === 0) {
		throw new Error("Selector cannot be empty.");
	}

	if (selector.startsWith("testid:")) {
		const value = selector.slice("testid:".length);
		if (value.length === 0) {
			throw new Error("Invalid testid selector. Use testid:<value>.");
		}
		return { kind: "testid", value: selector.slice("testid:".length) };
	}

	if (selector.startsWith("text:")) {
		const value = selector.slice("text:".length);
		if (value.length === 0) {
			throw new Error("Invalid text selector. Use text:<value>.");
		}
		return { kind: "text", value: selector.slice("text:".length) };
	}

	if (selector.startsWith("label:")) {
		const value = selector.slice("label:".length);
		if (value.length === 0) {
			throw new Error("Invalid label selector. Use label:<value>.");
		}
		return { kind: "label", value: selector.slice("label:".length) };
	}

	if (selector.startsWith("css:")) {
		const value = selector.slice("css:".length);
		if (value.length === 0) {
			throw new Error("Invalid css selector. Use css:<selector>.");
		}
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
	if (selector.startsWith("role:")) {
		throw new Error(
			'Invalid role selector. Use role:<name> or role:<name>[name="Accessible Name"].',
		);
	}

	const explicitPrefixMatch = selector.match(EXPLICIT_PREFIX_PATTERN);
	if (
		explicitPrefixMatch &&
		!KNOWN_PREFIXES.includes(
			explicitPrefixMatch[1].toLowerCase() as (typeof KNOWN_PREFIXES)[number],
		)
	) {
		throw new Error(
			`Unknown selector prefix "${explicitPrefixMatch[1]}". Use one of ${KNOWN_PREFIXES.join(", ")}, or prefix raw CSS with css:.`,
		);
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
