function describeElement(element: Element | null): string {
	if (!(element instanceof HTMLElement)) {
		return element ? element.tagName.toLowerCase() : "none";
	}

	const parts = [element.tagName.toLowerCase()];
	if (element.id) {
		parts.push(`#${element.id}`);
	}
	if (element.getAttribute("role")) {
		parts.push(`[role=${element.getAttribute("role")}]`);
	}
	if (element.getAttribute("data-state")) {
		parts.push(`[data-state=${element.getAttribute("data-state")}]`);
	}
	if (element instanceof HTMLInputElement && element.type) {
		parts.push(`[type=${element.type}]`);
	}
	if (element instanceof HTMLTextAreaElement) {
		parts.push("[textarea]");
	}
	if (element.tabIndex >= 0) {
		parts.push(`[tabindex=${element.tabIndex}]`);
	}

	return parts.join("");
}

export function domFocusSnapshot() {
	if (typeof document === "undefined") {
		return {
			documentHasFocus: null,
			activeElement: "unavailable",
		};
	}

	return {
		documentHasFocus: document.hasFocus(),
		activeElement: describeElement(document.activeElement),
	};
}
