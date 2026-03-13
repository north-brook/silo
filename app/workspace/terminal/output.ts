const DELETE_BYTE = 0x7f;
const BACKSPACE_ERASE_SEQUENCE = [0x08, 0x20, 0x08];

export function normalizeTerminalOutput(data: ArrayBuffer): Uint8Array {
	const bytes = new Uint8Array(data);
	if (!bytes.includes(DELETE_BYTE)) {
		return bytes;
	}

	const normalized: number[] = [];
	for (const byte of bytes) {
		if (byte === DELETE_BYTE) {
			normalized.push(...BACKSPACE_ERASE_SEQUENCE);
			continue;
		}
		normalized.push(byte);
	}

	return Uint8Array.from(normalized);
}
