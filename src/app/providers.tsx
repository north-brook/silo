import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { SessionHostProvider } from "@/workspaces/hosts/provider";
import { NewWorkspaceProvider } from "@/projects/sidebar/new-workspace";
import { OpenProjectProvider } from "@/projects/sidebar/open-project";
import { ProjectsSidebarProvider } from "@/projects/sidebar";
import { OverlayStateProvider } from "@/shared/ui/overlay-state";
import { Toaster } from "@/shared/ui/toaster";
import { TooltipProvider } from "@/shared/ui/tooltip";

export function AppProviders({ children }: { children: React.ReactNode }) {
	const [queryClient] = useState(() => new QueryClient());

	return (
		<QueryClientProvider client={queryClient}>
			<TooltipProvider
				delayDuration={0}
				skipDelayDuration={Infinity}
				disableHoverableContent
			>
				<OverlayStateProvider>
					<SessionHostProvider>
						<ProjectsSidebarProvider>
							<OpenProjectProvider>
								<NewWorkspaceProvider>{children}</NewWorkspaceProvider>
							</OpenProjectProvider>
						</ProjectsSidebarProvider>
					</SessionHostProvider>
				</OverlayStateProvider>
				<Toaster />
			</TooltipProvider>
		</QueryClientProvider>
	);
}
