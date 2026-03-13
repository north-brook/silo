import { describe, expect, test } from "bun:test";
import { sequenceForEvent } from "./bindings";

function event(overrides: Partial<KeyboardEvent> & { key: string }) {
	return {
		altKey: false,
		ctrlKey: false,
		metaKey: false,
		shiftKey: false,
		...overrides,
	};
}

describe("sequenceForEvent", () => {
	test("maps shift-enter to a literal newline", () => {
		expect(
			sequenceForEvent(
				event({
					key: "Enter",
					shiftKey: true,
				}),
			),
		).toBe("\n");
	});

	test("maps command-backspace to a private sequence", () => {
		expect(
			sequenceForEvent(
				event({
					key: "Backspace",
					metaKey: true,
				}),
			),
		).toBe("\u001b[1337;1u");
	});

	test("maps command-left and command-right to line navigation", () => {
		expect(
			sequenceForEvent(
				event({
					key: "ArrowLeft",
					metaKey: true,
				}),
			),
		).toBe("\u0001");

		expect(
			sequenceForEvent(
				event({
					key: "ArrowRight",
					metaKey: true,
				}),
			),
		).toBe("\u0005");
	});

	test("maps alt-left and alt-right to word navigation", () => {
		expect(
			sequenceForEvent(
				event({
					altKey: true,
					key: "ArrowLeft",
				}),
			),
		).toBe("\u001bb");

		expect(
			sequenceForEvent(
				event({
					altKey: true,
					key: "ArrowRight",
				}),
			),
		).toBe("\u001bf");
	});

	test("ignores unrecognized combinations", () => {
		expect(
			sequenceForEvent(
				event({
					key: "Backspace",
				}),
			),
		).toBeNull();
	});
});
