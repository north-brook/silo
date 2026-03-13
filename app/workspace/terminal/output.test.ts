import { expect, test } from "bun:test";
import { normalizeTerminalOutput } from "./output";

test("normalizeTerminalOutput leaves normal output untouched", () => {
	const input = Uint8Array.from([0x6c, 0x73, 0x0a]);
	const result = normalizeTerminalOutput(input.buffer);

	expect(Array.from(result)).toEqual([0x6c, 0x73, 0x0a]);
});

test("normalizeTerminalOutput expands delete bytes into erase sequences", () => {
	const input = Uint8Array.from([0x68, 0x68, 0x7f, 0x7f, 0x6c, 0x73]);
	const result = normalizeTerminalOutput(input.buffer);

	expect(Array.from(result)).toEqual([
		0x68,
		0x68,
		0x08,
		0x20,
		0x08,
		0x08,
		0x20,
		0x08,
		0x6c,
		0x73,
	]);
});
