import { Laptop, Terminal } from "lucide-react";
import { TerminalLoader } from "../components/terminal-loader";

function statusMessage(status: string): string {
	switch (status) {
		case "STAGING":
		case "PROVISIONING":
			return "Starting workspace...";
		case "STOPPING":
		case "SUSPENDING":
			return "Stopping workspace...";
		case "TERMINATED":
		case "STOPPED":
			return "Workspace is stopped";
		case "":
			return "Loading...";
		default:
			return status.charAt(0) + status.slice(1).toLowerCase();
	}
}

export function PendingWorkspace({
	isRunning,
	status,
}: {
	isRunning: boolean;
	status: string;
}) {
	return (
		<div className="flex-1 flex flex-col items-center justify-center p-6">
			<div className="w-full max-w-2xl">
				{isRunning ? (
					<div className="flex items-center gap-3">
						<button
							type="button"
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							<Terminal size={12} />
							Open Terminal
						</button>
						<button
							type="button"
							className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-text-muted hover:text-text transition-colors"
						>
							<Laptop size={12} />
							Open Desktop
						</button>
					</div>
				) : (
					<div className="flex items-center gap-2 px-2 py-1 text-[11px] text-text-muted">
						<TerminalLoader className="text-text-muted" />
						<span>{statusMessage(status)}</span>
					</div>
				)}
			</div>
		</div>
	);
}
