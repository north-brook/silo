import { describe, expect, test } from "bun:test";
import {
  type BranchWorkspace,
  type TemplateWorkspace,
  workspaceIsReady,
  workspaceLifecycleLabel,
  workspaceSessions,
} from "./api";

function baseWorkspace() {
  return {
    name: "demo-silo",
    project: "demo",
    last_active: null,
    active_session: null,
    created_at: "2026-03-20T00:00:00Z",
    status: "RUNNING",
    zone: "us-east1-b",
    lifecycle: {
      phase: "ready",
    },
  };
}

function session(
  type: "terminal" | "browser" | "file",
  attachmentId: string,
  name: string,
) {
  return {
    type,
    name,
    attachment_id: attachmentId,
    working: null,
    unread: null,
  };
}

describe("workspaceSessions", () => {
  test("returns template workspace sessions in the shared sorted order", () => {
    const workspace: TemplateWorkspace = {
      ...baseWorkspace(),
      template: true,
      terminals: [session("terminal", "terminal-200", "shell")],
      browsers: [session("browser", "browser-300", "docs")],
      files: [session("file", "file-100", "README.md")],
    };

    expect(
      workspaceSessions(workspace).map((entry) => entry.attachment_id),
    ).toEqual(["file-100", "terminal-200", "browser-300"]);
  });

  test("keeps branch workspace behavior unchanged", () => {
    const workspace: BranchWorkspace = {
      ...baseWorkspace(),
      branch: "feature/demo",
      target_branch: "main",
      unread: false,
      working: null,
      terminals: [session("terminal", "terminal-200", "shell")],
      browsers: [session("browser", "browser-300", "docs")],
      files: [session("file", "file-100", "README.md")],
    };

    expect(
      workspaceSessions(workspace).map((entry) => entry.attachment_id),
    ).toEqual(["file-100", "terminal-200", "browser-300"]);
  });
});

describe("workspace lifecycle labels", () => {
  test("treats workspace agent updates as non-ready", () => {
    const workspace: BranchWorkspace = {
      ...baseWorkspace(),
      lifecycle: {
        phase: "updating_workspace_agent",
      },
      branch: "feature/demo",
      target_branch: "main",
      unread: false,
      working: null,
      terminals: [],
      browsers: [],
      files: [],
    };

    expect(workspaceIsReady(workspace)).toBe(false);
    expect(workspaceLifecycleLabel(workspace)).toBe(
      "Updating workspace observer...",
    );
  });
});
