"use client";

import "./globals.css";
import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ProjectsBar } from "./components/projects-bar";
import { StatusBar } from "./components/status-bar";
import { TooltipProvider } from "./components/tooltip";
import { Toaster } from "./components/toaster";
import { useProjects } from "./hooks/use-projects";

function Shell({ children }: Readonly<{ children: React.ReactNode }>) {
	const projects = useProjects();
	const hasProjects = projects.data && projects.data.length > 0;

	return (
		<TooltipProvider delayDuration={300}>
			<ProjectsBar />
			<main className={`pb-6 h-full ${hasProjects ? "pl-48" : ""}`}>
				{children}
			</main>
			<StatusBar />
			<Toaster />
		</TooltipProvider>
	);
}

export default function RootLayout({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [queryClient] = useState(() => new QueryClient());

	useEffect(() => {
		import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
			getCurrentWindow().show();
		});
	}, []);

	return (
		<html lang="en">
			<QueryClientProvider client={queryClient}>
				<body>
					<Shell>{children}</Shell>
				</body>
			</QueryClientProvider>
		</html>
	);
}
