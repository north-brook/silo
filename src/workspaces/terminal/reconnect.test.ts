import { describe, expect, test } from "bun:test";
import {
	isRetryableTerminalTransportMessage,
	reconnectDelayMs,
} from "./reconnect";

describe("reconnectDelayMs", () => {
	test("starts with a short delay", () => {
		expect(reconnectDelayMs(0)).toBe(1000);
		expect(reconnectDelayMs(1)).toBe(1000);
	});

	test("caps at the longest configured delay", () => {
		expect(reconnectDelayMs(2)).toBe(2000);
		expect(reconnectDelayMs(3)).toBe(5000);
		expect(reconnectDelayMs(10)).toBe(15000);
	});
});

describe("isRetryableTerminalTransportMessage", () => {
	test("matches broken pipe style ssh failures", () => {
		expect(
			isRetryableTerminalTransportMessage(
				"client_loop: send disconnect: Broken pipe",
			),
		).toBe(true);
	});

	test("matches local network rebind failures", () => {
		expect(
			isRetryableTerminalTransportMessage(
				"Read from remote host 35.245.135.222: Can't assign requested address",
			),
		).toBe(true);
	});

	test("does not treat missing remote sessions as retryable", () => {
		expect(
			isRetryableTerminalTransportMessage("session not found: terminal-1"),
		).toBe(false);
	});
});
