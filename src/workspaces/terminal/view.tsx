import { useQuery } from "@tanstack/react-query";
import { FileClock } from "lucide-react";
import { useState } from "react";
import { invoke } from "@/shared/lib/invoke";
import {
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
} from "@/shared/ui/dialog";
import { Loader } from "@/shared/ui/loader";
import type { CloudSession } from "@/workspaces/hosts/model";
import { SessionViewport } from "@/workspaces/hosts/viewport";

interface TerminalTranscriptTailResult {
	content: string;
	truncated: boolean;
}

export function TerminalSessionView({
	session,
	skipInitialScrollback,
}: {
	session: CloudSession;
	skipInitialScrollback: boolean;
}) {
	const [transcriptOpen, setTranscriptOpen] = useState(false);
	const transcriptQuery = useQuery({
		queryKey: [
			"terminal_transcript_tail",
			session.workspace,
			session.attachmentId,
		],
		queryFn: () =>
			invoke<TerminalTranscriptTailResult>("terminal_read_transcript_tail", {
				attachmentId: session.attachmentId,
				maxBytes: 256 * 1024,
				workspace: session.workspace,
			}),
		enabled: transcriptOpen,
		gcTime: 60 * 1000,
	});

	return (
		<div className="relative h-full">
			<div className="absolute right-3 top-3 z-20">
				<button
					type="button"
					onClick={() => setTranscriptOpen(true)}
					className="inline-flex items-center gap-1.5 rounded border border-border-light bg-surface/90 px-2 py-1 text-xs text-text-muted backdrop-blur hover:text-text"
				>
					<FileClock size={12} />
					Transcript
				</button>
			</div>
			<SessionViewport
				workspace={session.workspace}
				activeSession={session}
				skipInitialScrollback={skipInitialScrollback}
			/>
			<Dialog open={transcriptOpen} onOpenChange={setTranscriptOpen}>
				<DialogContent className="max-w-4xl">
					<DialogHeader>
						<DialogTitle>Terminal Transcript</DialogTitle>
					</DialogHeader>
					<div className="rounded border border-border-light bg-bg">
						{transcriptQuery.isLoading ? (
							<div className="flex h-80 items-center justify-center">
								<Loader />
							</div>
						) : (
							<div className="max-h-[70vh] overflow-auto p-3">
								{transcriptQuery.data?.truncated && (
									<p className="mb-3 text-xs text-text-muted">
										Showing the most recent 256 KB of terminal output.
									</p>
								)}
								<pre className="whitespace-pre-wrap break-words font-mono text-xs text-text">
									{transcriptQuery.data?.content || "No transcript yet."}
								</pre>
							</div>
						)}
					</div>
				</DialogContent>
			</Dialog>
		</div>
	);
}
