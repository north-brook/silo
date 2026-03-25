import { describe, expect, test } from "bun:test";
import {
	clearWorkspaceLocalSession,
	type DisplayWorkspaceSession,
	defaultFileTabState,
	getWorkspaceLocalFileState,
	restoreWorkspaceLocalSession,
	updateWorkspaceLocalSessionStates,
	updateWorkspaceLocalSessions,
	type WorkspaceLocalFileState,
	type WorkspaceLocalSessionSnapshot,
} from "./local-state";

function fileSession(
	attachmentId: string,
	path: string,
	{
		persistentAttachmentId = null,
		preview = false,
	}: {
		persistentAttachmentId?: string | null;
		preview?: boolean;
	} = {},
): DisplayWorkspaceSession {
	return {
		type: "file",
		name: path.split("/").slice(-1)[0] || path,
		attachment_id: attachmentId,
		path,
		persistentAttachmentId,
		preview,
		working: null,
		unread: null,
	};
}

function workspaceState(
	sessions: DisplayWorkspaceSession[],
	sessionStates: Record<string, typeof defaultFileTabState>,
): WorkspaceLocalFileState {
	return { sessions, sessionStates };
}

function sessionSnapshot(
	session: DisplayWorkspaceSession,
	state: typeof defaultFileTabState = defaultFileTabState,
): WorkspaceLocalSessionSnapshot {
	return { session, state };
}

describe("workspace local file state", () => {
	test("updates local sessions only for the targeted workspace", () => {
		const alpha = workspaceState(
			[fileSession("file-alpha-1", "docs/alpha.md", { preview: true })],
			{
				"file-alpha-1": {
					...defaultFileTabState,
					dirty: true,
				},
			},
		);
		const beta = workspaceState([fileSession("file-beta-1", "docs/beta.md")], {
			"file-beta-1": defaultFileTabState,
		});
		const next = updateWorkspaceLocalSessions(
			{
				alpha,
				beta,
			},
			"beta",
			(previous) => [...previous, fileSession("file-beta-2", "docs/todo.md")],
		);

		expect(next.alpha).toBe(alpha);
		expect(next.beta.sessions.map((session) => session.attachment_id)).toEqual([
			"file-beta-1",
			"file-beta-2",
		]);
		expect(next.beta.sessionStates).toBe(beta.sessionStates);
	});

	test("updates tab state only for the targeted workspace", () => {
		const alpha = workspaceState(
			[fileSession("file-alpha-1", "docs/alpha.md")],
			{
				"file-alpha-1": defaultFileTabState,
			},
		);
		const beta = workspaceState(
			[fileSession("file-beta-1", "docs/beta.md", { preview: true })],
			{
				"file-beta-1": defaultFileTabState,
			},
		);
		const next = updateWorkspaceLocalSessionStates(
			{
				alpha,
				beta,
			},
			"beta",
			(previous) => ({
				...previous,
				"file-beta-1": {
					...defaultFileTabState,
					conflicted: true,
				},
			}),
		);

		expect(next.alpha).toBe(alpha);
		expect(next.beta.sessions).toBe(beta.sessions);
		expect(next.beta.sessionStates["file-beta-1"]).toEqual({
			...defaultFileTabState,
			conflicted: true,
		});
	});

	test("clears one workspace without touching the others", () => {
		const alpha = workspaceState(
			[fileSession("file-alpha-1", "docs/alpha.md", { preview: true })],
			{
				"file-alpha-1": {
					...defaultFileTabState,
					dirty: true,
				},
			},
		);
		const beta = workspaceState([fileSession("file-beta-1", "docs/beta.md")], {
			"file-beta-1": defaultFileTabState,
		});
		const next = clearWorkspaceLocalSession(
			{
				alpha,
				beta,
			},
			"beta",
			"file-beta-1",
		);

		expect(next.alpha).toBe(alpha);
		expect(next.beta).toBeUndefined();
		expect(getWorkspaceLocalFileState(next, "alpha")).toBe(alpha);
		expect(getWorkspaceLocalFileState(next, "beta").sessions).toEqual([]);
	});

	test("restores a cleared local session with its tab state", () => {
		const session = fileSession("file-beta-1", "docs/beta.md", {
			persistentAttachmentId: "file-remote-1",
		});
		const cleared = clearWorkspaceLocalSession(
			{
				beta: workspaceState([session], {
					"file-beta-1": {
						...defaultFileTabState,
						dirty: true,
					},
				}),
			},
			"beta",
			"file-beta-1",
		);

		const restored = restoreWorkspaceLocalSession(
			cleared,
			"beta",
			sessionSnapshot(session, {
				...defaultFileTabState,
				dirty: true,
			}),
		);

		expect(getWorkspaceLocalFileState(restored, "beta").sessions).toEqual([
			session,
		]);
		expect(
			getWorkspaceLocalFileState(restored, "beta").sessionStates["file-beta-1"],
		).toEqual({
			...defaultFileTabState,
			dirty: true,
		});
	});
});
