import type { Metadata } from "next";
import { JetBrains_Mono } from "next/font/google";
import "./globals.css";

const mono = JetBrains_Mono({
	subsets: ["latin"],
	weight: ["400", "500"],
	variable: "--font-mono-face",
});

export const metadata: Metadata = {
	title: "Silo - Orchestrate Parallel AI Agents on Cloud VMs",
	description:
		"Orchestrate parallel AI agents on cloud VMs from your Mac. Each task gets its own VM with dedicated ports, docker images, and isolated services. Open source — bring your own cloud & keys.",
	openGraph: {
		title: "Silo - Orchestrate Parallel AI Agents on Cloud VMs",
		description:
			"Orchestrate parallel AI agents on cloud VMs from your Mac. Each task gets its own VM with dedicated ports, docker images, and isolated services.",
		type: "website",
	},
};

export default function RootLayout({
	children,
}: Readonly<{
	children: React.ReactNode;
}>) {
	return (
		<html lang="en">
			<body className={mono.variable}>{children}</body>
		</html>
	);
}
