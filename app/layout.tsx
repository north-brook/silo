"use client";

import "./globals.css";
import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

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
					<header className="z-50 absolute top-0 left-0 right-0 h-8 w-full border-b border-border-light">
						<div data-tauri-drag-region className="absolute inset-0" />
					</header>
					{children}
					<footer className="fixed bottom-0 left-0 right-0 h-6 flex items-center px-3 text-[11px] text-text-muted border-t border-border-light bg-bg">
						<span>Silo v0.1.0</span>
					</footer>
				</body>
			</QueryClientProvider>
		</html>
	);
}
