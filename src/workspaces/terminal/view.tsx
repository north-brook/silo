"use client";

import type { CloudSession } from "@/workspaces/hosts/model";
import { SessionViewport } from "@/workspaces/hosts/viewport";

export function TerminalSessionView({
	session,
	skipInitialScrollback,
}: {
	session: CloudSession;
	skipInitialScrollback: boolean;
}) {
	return (
		<SessionViewport
			workspace={session.workspace}
			activeSession={session}
			skipInitialScrollback={skipInitialScrollback}
		/>
	);
}
