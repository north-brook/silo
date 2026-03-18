import { Navigate, Route, Routes } from "react-router-dom";
import HomePage from "@/dashboard/page";
import { WorkspaceShell } from "@/workspaces/layout/shell";
import WorkspacePage from "@/workspaces/routes/page";
import {
	WorkspaceBrowserSessionPage,
	WorkspaceTerminalSessionPage,
} from "@/workspaces/routes/session-page";
import { AppShell } from "@/app/shell";

export function AppRouter() {
	return (
		<Routes>
			<Route element={<AppShell />}>
				<Route index element={<HomePage />} />
				<Route
					path="projects/:project/workspaces/:workspace"
					element={<WorkspaceShell />}
				>
					<Route index element={<WorkspacePage />} />
					<Route
						path="browser/:attachmentId"
						element={<WorkspaceBrowserSessionPage />}
					/>
					<Route
						path="terminal/:attachmentId"
						element={<WorkspaceTerminalSessionPage />}
					/>
				</Route>
				<Route path="*" element={<Navigate to="/" replace />} />
			</Route>
		</Routes>
	);
}
