"use client";

import { useEffect, useState } from "react";

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function Loader({ className }: { className?: string }) {
	const [frame, setFrame] = useState(0);

	useEffect(() => {
		const id = setInterval(() => setFrame((f) => (f + 1) % FRAMES.length), 80);
		return () => clearInterval(id);
	}, []);

	return (
		<span className={`shrink-0 leading-none inline-flex items-center justify-center aspect-square ${className ?? "text-text-muted"}`}>
			{FRAMES[frame]}
		</span>
	);
}
