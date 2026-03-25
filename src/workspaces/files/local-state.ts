import type { WorkspaceSession } from "../api";

export interface FileTabState {
	conflicted: boolean;
	dirty: boolean;
	saving: boolean;
}

export interface DisplayWorkspaceSession extends WorkspaceSession {
	persistentAttachmentId?: string | null;
	preview?: boolean;
}

export interface WorkspaceLocalFileState {
	sessions: DisplayWorkspaceSession[];
	sessionStates: Record<string, FileTabState>;
}

export interface WorkspaceLocalSessionSnapshot {
	session: DisplayWorkspaceSession;
	state: FileTabState;
}

export const defaultFileTabState: FileTabState = {
	conflicted: false,
	dirty: false,
	saving: false,
};

export const emptyWorkspaceLocalFileState: WorkspaceLocalFileState = {
	sessions: [],
	sessionStates: {},
};

export function getWorkspaceLocalFileState(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
): WorkspaceLocalFileState {
	if (!workspace) {
		return emptyWorkspaceLocalFileState;
	}
	return stateByWorkspace[workspace] ?? emptyWorkspaceLocalFileState;
}

export function updateWorkspaceLocalFileState(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
	updater: (previous: WorkspaceLocalFileState) => WorkspaceLocalFileState,
): Record<string, WorkspaceLocalFileState> {
	if (!workspace) {
		return stateByWorkspace;
	}

	const previousWorkspaceState =
		stateByWorkspace[workspace] ?? emptyWorkspaceLocalFileState;
	const nextWorkspaceState = updater(previousWorkspaceState);
	if (
		nextWorkspaceState.sessions === previousWorkspaceState.sessions &&
		nextWorkspaceState.sessionStates === previousWorkspaceState.sessionStates
	) {
		return stateByWorkspace;
	}

	if (workspaceLocalFileStateIsEmpty(nextWorkspaceState)) {
		if (!(workspace in stateByWorkspace)) {
			return stateByWorkspace;
		}
		const nextStateByWorkspace = { ...stateByWorkspace };
		delete nextStateByWorkspace[workspace];
		return nextStateByWorkspace;
	}

	return {
		...stateByWorkspace,
		[workspace]: nextWorkspaceState,
	};
}

export function updateWorkspaceLocalSessions(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
	updater: (previous: DisplayWorkspaceSession[]) => DisplayWorkspaceSession[],
): Record<string, WorkspaceLocalFileState> {
	return updateWorkspaceLocalFileState(
		stateByWorkspace,
		workspace,
		(previousWorkspaceState) => {
			const nextSessions = updater(previousWorkspaceState.sessions);
			if (nextSessions === previousWorkspaceState.sessions) {
				return previousWorkspaceState;
			}
			return {
				...previousWorkspaceState,
				sessions: nextSessions,
			};
		},
	);
}

export function updateWorkspaceLocalSessionStates(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
	updater: (
		previous: Record<string, FileTabState>,
	) => Record<string, FileTabState>,
): Record<string, WorkspaceLocalFileState> {
	return updateWorkspaceLocalFileState(
		stateByWorkspace,
		workspace,
		(previousWorkspaceState) => {
			const nextSessionStates = updater(previousWorkspaceState.sessionStates);
			if (nextSessionStates === previousWorkspaceState.sessionStates) {
				return previousWorkspaceState;
			}
			return {
				...previousWorkspaceState,
				sessionStates: nextSessionStates,
			};
		},
	);
}

export function clearWorkspaceLocalSession(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
	attachmentId: string,
): Record<string, WorkspaceLocalFileState> {
	return updateWorkspaceLocalFileState(
		stateByWorkspace,
		workspace,
		(previousWorkspaceState) => {
			const nextSessions = previousWorkspaceState.sessions.filter(
				(session) => session.attachment_id !== attachmentId,
			);
			const removedSession =
				nextSessions.length !== previousWorkspaceState.sessions.length;
			const hasSessionState =
				attachmentId in previousWorkspaceState.sessionStates;
			if (!removedSession && !hasSessionState) {
				return previousWorkspaceState;
			}

			let nextSessionStates = previousWorkspaceState.sessionStates;
			if (hasSessionState) {
				nextSessionStates = { ...previousWorkspaceState.sessionStates };
				delete nextSessionStates[attachmentId];
			}

			return {
				sessions: nextSessions,
				sessionStates: nextSessionStates,
			};
		},
	);
}

export function restoreWorkspaceLocalSession(
	stateByWorkspace: Record<string, WorkspaceLocalFileState>,
	workspace: string | null | undefined,
	snapshot: WorkspaceLocalSessionSnapshot,
): Record<string, WorkspaceLocalFileState> {
	return updateWorkspaceLocalFileState(
		stateByWorkspace,
		workspace,
		(previousWorkspaceState) => {
			const existingIndex = previousWorkspaceState.sessions.findIndex(
				(session) => session.attachment_id === snapshot.session.attachment_id,
			);

			let nextSessions = previousWorkspaceState.sessions;
			if (existingIndex === -1) {
				nextSessions = [...previousWorkspaceState.sessions, snapshot.session];
			} else if (
				previousWorkspaceState.sessions[existingIndex] !== snapshot.session
			) {
				nextSessions = [...previousWorkspaceState.sessions];
				nextSessions[existingIndex] = snapshot.session;
			}

			const previousState =
				previousWorkspaceState.sessionStates[snapshot.session.attachment_id];
			const stateUnchanged =
				previousState?.conflicted === snapshot.state.conflicted &&
				previousState?.dirty === snapshot.state.dirty &&
				previousState?.saving === snapshot.state.saving;
			const nextSessionStates = stateUnchanged
				? previousWorkspaceState.sessionStates
				: {
						...previousWorkspaceState.sessionStates,
						[snapshot.session.attachment_id]: snapshot.state,
					};

			if (
				nextSessions === previousWorkspaceState.sessions &&
				nextSessionStates === previousWorkspaceState.sessionStates
			) {
				return previousWorkspaceState;
			}

			return {
				sessions: nextSessions,
				sessionStates: nextSessionStates,
			};
		},
	);
}

function workspaceLocalFileStateIsEmpty(state: WorkspaceLocalFileState) {
	return (
		state.sessions.length === 0 && Object.keys(state.sessionStates).length === 0
	);
}
