import { describe, expect, test } from "bun:test";
import {
	TERMINAL_ASSISTANT_COMPLETION_REFRESH_DELAY_MS,
	TERMINAL_COMMAND_STATE_REFRESH_DELAY_MS,
	terminalInputContainsSubmit,
} from "./refresh";

describe("terminalInputContainsSubmit", () => {
	test("treats carriage-return terminated input as a submit", () => {
		expect(terminalInputContainsSubmit("codex\r")).toBe(true);
		expect(
			terminalInputContainsSubmit(Uint8Array.from([99, 111, 100, 101, 120, 13])),
		).toBe(true);
	});

	test("treats newline-terminated pasted input as a submit", () => {
		expect(terminalInputContainsSubmit("bun run dev\n")).toBe(true);
	});

	test("ignores plain typing without a submit key", () => {
		expect(terminalInputContainsSubmit("codex")).toBe(false);
		expect(
			terminalInputContainsSubmit(Uint8Array.from([99, 111, 100, 101, 120])),
		).toBe(false);
	});
});

describe("terminal refresh delays", () => {
	test("uses a short submit refresh and a buffered assistant completion refresh", () => {
		expect(TERMINAL_COMMAND_STATE_REFRESH_DELAY_MS).toBe(200);
		expect(TERMINAL_ASSISTANT_COMPLETION_REFRESH_DELAY_MS).toBe(6500);
	});
});
