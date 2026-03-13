"use client";

import "./globals.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { initializeFrontendLogging } from "../lib/invoke";
import { ProjectsBar } from "./components/projects-bar";
import { Toaster } from "./components/toaster";
import { TooltipProvider } from "./components/tooltip";

export default function RootLayout({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [queryClient] = useState(() => new QueryClient());

	useEffect(() => {
		initializeFrontendLogging();
		import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
			getCurrentWindow().show();
		});
	}, []);

	return (
		<html lang="en">
			<QueryClientProvider client={queryClient}>
				<TooltipProvider
					delayDuration={0}
					skipDelayDuration={Infinity}
					disableHoverableContent
				>
					<body className="flex flex-col h-screen overflow-hidden">
						<div className="flex flex-1 min-h-0">
							<ProjectsBar />
							<main className="flex-1 min-w-0 overflow-auto flex flex-col">
								{children}
							</main>
						</div>
						<Toaster />
					</body>
				</TooltipProvider>
			</QueryClientProvider>
		</html>
	);
}
