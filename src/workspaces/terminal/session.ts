export type AssistantTerminalModel = "codex" | "claude";

const ASSISTANT_PROXY_PROVIDER_RE =
	/\bassistant-proxy\b[\s\S]*?--provider\s+(?:"([^"]+)"|'([^']+)'|([^\s]+))/i;

function normalizeAssistantToken(
	value: string | undefined,
): AssistantTerminalModel | null {
	const normalized = value?.trim().replace(/^['"]|['"]$/g, "").toLowerCase();
	if (normalized === "cc" || normalized === "claude") {
		return "claude";
	}
	if (normalized === "codex") {
		return "codex";
	}
	return null;
}

export function assistantTerminalModel(
	name: string,
): AssistantTerminalModel | null {
	const trimmed = name.trim();
	const lower = trimmed.toLowerCase();
	const assistantProxyMatch = trimmed.match(ASSISTANT_PROXY_PROVIDER_RE);
	const assistantProxyProvider = normalizeAssistantToken(
		assistantProxyMatch?.[1] ??
			assistantProxyMatch?.[2] ??
			assistantProxyMatch?.[3],
	);
	if (assistantProxyProvider) {
		return assistantProxyProvider;
	}
	const [token, ...rest] = trimmed.split(/\s+/);
	const normalizedAssistant =
		token?.toLowerCase() === "silo"
			? normalizeAssistantToken(rest[0])
			: normalizeAssistantToken(token);
	if (normalizedAssistant) {
		return normalizedAssistant;
	}
	if (lower.startsWith("command claude")) {
		return "claude";
	}
	if (lower.startsWith("command codex")) {
		return "codex";
	}
	return null;
}
