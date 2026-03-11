"use client";

import Image from "next/image";
import { FolderOpen } from "lucide-react";

export default function HomePage() {
	return (
		<main className="flex flex-col items-center justify-center h-full gap-6">
			<Image
				src="/logo.svg"
				height={32}
				width={160}
				alt="Silo"
				className="h-8 w-auto"
				draggable={false}
			/>
			<button
				type="button"
				className="flex items-center gap-2 px-4 py-2 text-sm rounded-lg bg-btn border border-border-light text-text-bright hover:bg-btn-hover hover:border-border-hover transition-colors"
			>
				<FolderOpen size={16} />
				Open Project
			</button>
		</main>
	);
}
