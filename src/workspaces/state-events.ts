import {
	isTemplateWorkspace,
	type WorkspaceLifecycle,
	type WorkspaceTemplateOperationState,
	type Workspace,
	type WorkspaceSession,
} from "@/workspaces/api";

export interface WorkspaceStateEventPayload {
	workspace: string;
	clearedActiveSession: boolean;
	removedSessionAttachmentId?: string | null;
	removedSessionKind?: string | null;
	templateOperation?: WorkspaceTemplateOperationState | null;
	lifecycle?: WorkspaceLifecycle | null;
}

export function applyWorkspaceStateEventToWorkspace(
	current: Workspace | null | undefined,
	event: WorkspaceStateEventPayload,
): Workspace | null | undefined {
	if (!current) {
		return current;
	}

	const nextActiveSession = event.clearedActiveSession
		? null
		: (current.active_session ?? null);
	const hasTemplateOperationUpdate = event.templateOperation !== undefined;
	const nextTemplateOperation =
		hasTemplateOperationUpdate && isTemplateWorkspace(current)
			? (event.templateOperation ?? null)
			: isTemplateWorkspace(current)
				? (current.template_operation ?? null)
				: null;
	const hasLifecycleUpdate = event.lifecycle !== undefined;
	const nextLifecycle =
		hasLifecycleUpdate && event.lifecycle ? event.lifecycle : current.lifecycle;

	if (!event.removedSessionAttachmentId || !event.removedSessionKind) {
		if (
			nextActiveSession === current.active_session &&
			(!hasLifecycleUpdate || nextLifecycle === current.lifecycle) &&
			(!isTemplateWorkspace(current) ||
				!hasTemplateOperationUpdate ||
				nextTemplateOperation === current.template_operation)
		) {
			return current;
		}
		if (isTemplateWorkspace(current)) {
			return {
				...current,
				active_session: nextActiveSession,
				lifecycle: nextLifecycle,
				template_operation: nextTemplateOperation,
			};
		}
		return {
			...current,
			active_session: nextActiveSession,
			lifecycle: nextLifecycle,
		};
	}

	const removeSession = (sessions: WorkspaceSession[]) =>
		sessions.filter(
			(session) =>
				!(
					session.type === event.removedSessionKind &&
					session.attachment_id === event.removedSessionAttachmentId
				),
		);

	const nextTerminals = removeSession(current.terminals);
	const nextBrowsers = removeSession(current.browsers);
	const nextFiles = removeSession(current.files);
	const assistantPresent = nextTerminals.some(
		(session) => session.working != null || session.unread != null,
	);
	const nextWorking = assistantPresent
		? nextTerminals.some((session) => session.working === true)
		: null;
	const nextUnread = nextTerminals.some((session) => session.unread === true);

	if (
		nextTerminals.length === current.terminals.length &&
		nextBrowsers.length === current.browsers.length &&
		nextFiles.length === current.files.length &&
		nextActiveSession === current.active_session &&
		(!hasLifecycleUpdate || nextLifecycle === current.lifecycle) &&
		(!isTemplateWorkspace(current) ||
			!hasTemplateOperationUpdate ||
			nextTemplateOperation === current.template_operation)
	) {
		return current;
	}

	if (isTemplateWorkspace(current)) {
		return {
			...current,
			active_session: nextActiveSession,
			terminals: nextTerminals,
			browsers: nextBrowsers,
			files: nextFiles,
			lifecycle: nextLifecycle,
			template_operation: nextTemplateOperation,
		};
	}

	return {
		...current,
		active_session: nextActiveSession,
		terminals: nextTerminals,
		browsers: nextBrowsers,
		files: nextFiles,
		lifecycle: nextLifecycle,
		unread: nextUnread,
		working: nextWorking,
	};
}

export function removeWorkspaceSessionFromWorkspace(
	current: Workspace | null | undefined,
	{
		attachmentId,
		kind,
	}: {
		attachmentId: string;
		kind: string;
	},
): Workspace | null | undefined {
	if (!current) {
		return current;
	}

	const trimmedAttachmentId = attachmentId.trim();
	const trimmedKind = kind.trim();
	if (!trimmedAttachmentId || !trimmedKind) {
		return current;
	}

	return applyWorkspaceStateEventToWorkspace(current, {
		workspace: current.name,
		clearedActiveSession:
			current.active_session?.type === trimmedKind &&
			current.active_session?.attachment_id === trimmedAttachmentId,
		removedSessionAttachmentId: trimmedAttachmentId,
		removedSessionKind: trimmedKind,
	});
}
