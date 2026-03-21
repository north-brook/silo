import { describe, expect, test } from "bun:test";
import type { BranchWorkspace, WorkspaceSession } from "@/workspaces/api";
import {
	removeWorkspaceSessionFromWorkspace,
} from "@/workspaces/state-events";

function session(
	type: WorkspaceSession["type"],
	attachmentId: string,
	overrides: Partial<WorkspaceSession> = {},
): WorkspaceSession {
	return {
		type,
		name: attachmentId,
		attachment_id: attachmentId,
		path: null,
		url: null,
		logical_url: null,
		resolved_url: null,
		title: null,
		favicon_url: null,
		can_go_back: null,
		can_go_forward: null,
		working: null,
		unread: null,
		...overrides,
	};
}

function workspace(
	overrides: Partial<BranchWorkspace> = {},
): BranchWorkspace {
	return {
		name: "demo-silo",
		project: "demo",
		branch: "main",
		target_branch: "main",
		last_active: null,
		active_session: null,
		created_at: "2026-03-20T00:00:00Z",
		status: "RUNNING",
		zone: "us-east1-b",
		lifecycle: {
			phase: "ready",
		},
		unread: false,
		working: null,
		terminals: [],
		browsers: [],
		files: [],
		...overrides,
	};
}

describe("removeWorkspaceSessionFromWorkspace", () => {
	test("removes the last session and clears the active session", () => {
		const current = workspace({
			active_session: {
				type: "browser",
				attachment_id: "browser-1",
			},
			browsers: [session("browser", "browser-1")],
		});

		const next = removeWorkspaceSessionFromWorkspace(current, {
			kind: "browser",
			attachmentId: "browser-1",
		});

		expect(next).not.toBeNull();
		expect(next?.active_session).toBeNull();
		expect(next?.browsers).toEqual([]);
		expect(next?.terminals).toEqual([]);
		expect(next?.files).toEqual([]);
	});

	test("keeps the active session when removing a background tab", () => {
		const current = workspace({
			active_session: {
				type: "terminal",
				attachment_id: "terminal-2",
			},
			terminals: [session("terminal", "terminal-2")],
			browsers: [session("browser", "browser-1")],
		});

		const next = removeWorkspaceSessionFromWorkspace(current, {
			kind: "browser",
			attachmentId: "browser-1",
		});

		expect(next).not.toBeNull();
		expect(next?.active_session).toEqual(current.active_session);
		expect(next?.terminals).toEqual(current.terminals);
		expect(next?.browsers).toEqual([]);
	});
});
