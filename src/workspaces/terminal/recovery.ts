export const STALE_ATTACHMENT_RECONNECT_MESSAGE =
	"Terminal connection was lost. Attempting to reconnect.";

export const STALE_ATTACHMENT_RESUME_NOTICE =
	"[connection lost, attempting to resume]";

const MISSING_LOCAL_ATTACHMENT_PATTERN = "terminal attachment not found";

export function isMissingLocalTerminalAttachmentMessage(message: string) {
	return message.toLowerCase().includes(MISSING_LOCAL_ATTACHMENT_PATTERN);
}
