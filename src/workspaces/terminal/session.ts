export type AssistantTerminalModel = "codex" | "claude";

export function assistantTerminalModel(
	name: string,
): AssistantTerminalModel | null {
	const trimmed = name.trim();
	const lower = trimmed.toLowerCase();
	const [token, ...rest] = trimmed.split(/\s+/);
	const normalizedToken = token?.toLowerCase() ?? "";
	const normalizedAssistant =
		normalizedToken === "silo"
			? (rest[0]?.toLowerCase() ?? "")
			: normalizedToken;

	if (normalizedAssistant === "cc" || normalizedAssistant === "claude") {
		return "claude";
	}
	if (normalizedAssistant === "codex" || lower.startsWith("command codex")) {
		return "codex";
	}
	return null;
}
