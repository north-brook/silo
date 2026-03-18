"use client";

import { useEffect } from "react";
import { Outlet } from "react-router-dom";
import { ProjectsSidebar } from "@/projects/sidebar";
import { initializeFrontendLogging } from "@/shared/lib/invoke";

export function AppShell() {
	useEffect(() => {
		initializeFrontendLogging();
		import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
			getCurrentWindow().show();
		});
	}, []);

	return (
		<div className="flex h-screen overflow-hidden">
			<ProjectsSidebar />
			<main className="flex-1 min-w-0 overflow-hidden flex flex-col">
				<Outlet />
			</main>
		</div>
	);
}
