import { Navigate, Route, Routes } from "react-router-dom";
import HomePage from "@/dashboard/page";
import { WorkspaceShell } from "@/workspaces/layout/shell";
import ResumingPage from "@/workspaces/routes/resuming-page";
import SavingPage from "@/workspaces/routes/saving-page";
import WorkspacePage from "@/workspaces/routes/page";
import WorkspaceSessionPage from "@/workspaces/routes/session-page";
import { AppShell } from "@/app/shell";

export function AppRouter() {
	return (
		<Routes>
			<Route element={<AppShell />}>
				<Route index element={<HomePage />} />
				<Route path="workspace" element={<WorkspaceShell />}>
					<Route index element={<WorkspacePage />} />
					<Route path="session" element={<WorkspaceSessionPage />} />
					<Route path="saving" element={<SavingPage />} />
					<Route path="resuming" element={<ResumingPage />} />
				</Route>
				<Route path="*" element={<Navigate to="/" replace />} />
			</Route>
		</Routes>
	);
}
