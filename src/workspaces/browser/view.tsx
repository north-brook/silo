import type { CloudSession } from "@/workspaces/hosts/model";
import { SessionViewport } from "@/workspaces/hosts/viewport";
import { BrowserSessionHeader } from "@/workspaces/browser/header";

export function BrowserSessionView({
	session,
	autoFocusAddress,
	onChanged,
}: {
	session: CloudSession;
	autoFocusAddress?: boolean;
	onChanged: () => void;
}) {
	const hasUrl = !!session.url;

	return (
		<div className="flex-1 min-h-0 bg-surface flex flex-col">
			<BrowserSessionHeader
				key={`${session.workspace}:${session.attachmentId}`}
				session={session}
				autoFocusAddress={autoFocusAddress === true || !hasUrl}
				onChanged={onChanged}
			/>
			{hasUrl && (
				<SessionViewport
					workspace={session.workspace}
					activeSession={session}
					skipInitialScrollback={false}
				/>
			)}
		</div>
	);
}
