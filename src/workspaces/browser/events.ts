import { browserSessionHref } from "@/workspaces/routes/paths";

export interface BrowserStateEventPayload {
	workspace: string;
	attachmentId?: string | null;
	popupAttachmentId?: string | null;
}

export function popupBrowserSessionHrefForEvent(
	event: BrowserStateEventPayload,
	{
		project,
		workspaceName,
	}: {
		project: string;
		workspaceName: string;
	},
) {
	if (event.workspace !== workspaceName) {
		return null;
	}

	const attachmentId = event.popupAttachmentId?.trim();
	if (!attachmentId) {
		return null;
	}

	return browserSessionHref({
		project,
		workspace: workspaceName,
		attachmentId,
	});
}
