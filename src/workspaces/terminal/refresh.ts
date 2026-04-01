export const TERMINAL_COMMAND_STATE_REFRESH_DELAY_MS = 200;

// Match the assistant proxy's 6s idle completion timeout with a small buffer.
export const TERMINAL_ASSISTANT_COMPLETION_REFRESH_DELAY_MS = 6500;

export function terminalInputContainsSubmit(
	data: string | Uint8Array,
): boolean {
	if (typeof data === "string") {
		return data.includes("\r") || data.includes("\n");
	}

	for (const byte of data) {
		if (byte === 0x0d || byte === 0x0a) {
			return true;
		}
	}

	return false;
}
