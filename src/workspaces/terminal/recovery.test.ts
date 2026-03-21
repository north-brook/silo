import { describe, expect, test } from "bun:test";
import {
	isMissingLocalTerminalAttachmentMessage,
	STALE_ATTACHMENT_RECONNECT_MESSAGE,
	STALE_ATTACHMENT_RESUME_NOTICE,
} from "./recovery";

describe("isMissingLocalTerminalAttachmentMessage", () => {
	test("matches stale local attachment errors", () => {
		expect(
			isMissingLocalTerminalAttachmentMessage(
				"terminal attachment not found: abc123",
			),
		).toBe(true);
	});

	test("is case insensitive", () => {
		expect(
			isMissingLocalTerminalAttachmentMessage(
				"Terminal Attachment Not Found: abc123",
			),
		).toBe(true);
	});

	test("does not match unrelated transport errors", () => {
		expect(
			isMissingLocalTerminalAttachmentMessage(
				"Connection reset by 35.245.76.210 port 22",
			),
		).toBe(false);
	});
});

describe("stale attachment recovery copy", () => {
	test("uses a reconnect message that explains the action", () => {
		expect(STALE_ATTACHMENT_RECONNECT_MESSAGE).toContain("reconnect");
	});

	test("uses a resume notice for terminal output", () => {
		expect(STALE_ATTACHMENT_RESUME_NOTICE).toContain("resume");
	});
});
