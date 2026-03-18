import type { CloudSession } from "@/workspaces/hosts/model";
import { SessionViewport } from "@/workspaces/hosts/viewport";
import { BrowserSessionHeader } from "@/workspaces/browser/header";

export function BrowserSessionView({
	session,
	onChanged,
}: {
	session: CloudSession;
	onChanged: () => void;
}) {
	const hasUrl = !!session.url;

	return (
		<div className="flex-1 min-h-0 bg-surface flex flex-col">
			<BrowserSessionHeader
				key={`${session.workspace}:${session.attachmentId}`}
				session={session}
				autoFocusAddress={!hasUrl}
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
