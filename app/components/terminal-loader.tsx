"use client";

import { useState, useEffect } from "react";

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function TerminalLoader({ className }: { className?: string }) {
	const [frame, setFrame] = useState(0);

	useEffect(() => {
		const id = setInterval(() => setFrame((f) => (f + 1) % FRAMES.length), 80);
		return () => clearInterval(id);
	}, []);

	return <span className={`shrink-0 ${className ?? "text-accent"}`}>{FRAMES[frame]}</span>;
}
