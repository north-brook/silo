import { describe, expect, test } from "bun:test";
import type {
	BranchWorkspace,
	TemplateWorkspace,
	WorkspaceSession,
} from "@/workspaces/api";
import {
	applyWorkspaceStateEventToWorkspace,
	applyWorkspaceStateEventToWorkspaces,
	removeWorkspaceSessionFromWorkspace,
	replaceWorkspaceInWorkspaces,
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

function workspace(overrides: Partial<BranchWorkspace> = {}): BranchWorkspace {
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

function templateWorkspace(
	overrides: Partial<TemplateWorkspace> = {},
): TemplateWorkspace {
	return {
		name: "demo-silo-template",
		project: "demo",
		last_active: null,
		active_session: {
			type: "terminal",
			attachment_id: "terminal-1",
		},
		created_at: "2026-03-20T00:00:00Z",
		status: "RUNNING",
		zone: "us-east1-b",
		lifecycle: {
			phase: "ready",
		},
		terminals: [session("terminal", "terminal-1")],
		browsers: [],
		files: [],
		template: true,
		template_operation: null,
		...overrides,
	};
}

describe("applyWorkspaceStateEventToWorkspace", () => {
	test("applies a template operation update without removing sessions", () => {
		const current = templateWorkspace();
		const next = applyWorkspaceStateEventToWorkspace(current, {
			workspace: current.name,
			clearedActiveSession: false,
			templateOperation: {
				kind: "save",
				phase: "waiting_for_template_ready",
				detail: "Waiting for template workspace bootstrap",
				last_error: null,
				updated_at: "2026-03-22T12:00:00Z",
				snapshot_name: "demo-template-2026-03-22",
			},
		});

		expect(next).not.toBeNull();
		expect(next).toMatchObject({
			active_session: current.active_session,
			terminals: current.terminals,
		});
		expect("template_operation" in (next ?? {})).toBe(true);
		expect((next as TemplateWorkspace | null)?.template_operation).toEqual({
			kind: "save",
			phase: "waiting_for_template_ready",
			detail: "Waiting for template workspace bootstrap",
			last_error: null,
			updated_at: "2026-03-22T12:00:00Z",
			snapshot_name: "demo-template-2026-03-22",
		});
	});

	test("applies a lifecycle update without removing sessions", () => {
		const current = workspace();
		const next = applyWorkspaceStateEventToWorkspace(current, {
			workspace: current.name,
			clearedActiveSession: false,
			lifecycle: {
				phase: "bootstrapping",
				detail: "Preparing repository, credentials, and tools",
				last_error: null,
				updated_at: "2026-03-23T00:00:00Z",
			},
		});

		expect(next).not.toBeNull();
		expect(next?.terminals).toEqual(current.terminals);
		expect(next?.browsers).toEqual(current.browsers);
		expect(next?.files).toEqual(current.files);
		expect(next?.lifecycle).toEqual({
			phase: "bootstrapping",
			detail: "Preparing repository, credentials, and tools",
			last_error: null,
			updated_at: "2026-03-23T00:00:00Z",
		});
	});
});

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

describe("applyWorkspaceStateEventToWorkspaces", () => {
	test("updates the matching workspace in a cached collection", () => {
		const current = [
			workspace({
				name: "alpha-silo",
				lifecycle: { phase: "ready" },
			}),
			workspace({
				name: "beta-silo",
				lifecycle: { phase: "ready" },
			}),
		];

		const next = applyWorkspaceStateEventToWorkspaces(current, {
			workspace: "beta-silo",
			clearedActiveSession: false,
			lifecycle: {
				phase: "bootstrapping",
				detail: "Preparing repository",
				last_error: null,
				updated_at: "2026-03-23T00:00:00Z",
			},
		});

		expect(next).not.toBe(current);
		expect(next?.[0]).toBe(current[0]);
		expect(next?.[1]?.lifecycle.phase).toBe("bootstrapping");
	});
});

describe("replaceWorkspaceInWorkspaces", () => {
	test("replaces the matching workspace without touching neighbors", () => {
		const current = [
			workspace({ name: "alpha-silo", lifecycle: { phase: "ready" } }),
			workspace({ name: "beta-silo", lifecycle: { phase: "ready" } }),
		];
		const replacement = workspace({
			name: "beta-silo",
			lifecycle: { phase: "waiting_for_agent" },
		});

		const next = replaceWorkspaceInWorkspaces(current, replacement);

		expect(next).not.toBe(current);
		expect(next?.[0]).toBe(current[0]);
		expect(next?.[1]).toEqual(replacement);
	});
});
