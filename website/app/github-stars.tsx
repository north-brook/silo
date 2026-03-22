"use client";

import { useEffect, useState } from "react";

export function GitHubStars({ repo }: { repo: string }) {
	const [stars, setStars] = useState<number | null>(null);

	useEffect(() => {
		fetch(`https://api.github.com/repos/${repo}`)
			.then((res) => res.json())
			.then((data) => {
				if (typeof data.stargazers_count === "number") {
					setStars(data.stargazers_count);
				}
			})
			.catch(() => {});
	}, [repo]);

	return (
		<a
			href={`https://github.com/${repo}`}
			target="_blank"
			rel="noopener noreferrer"
			className="inline-flex items-center gap-2 font-mono text-[12px] text-text-muted transition-colors hover:text-text-bright"
		>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
				<title>GitHub</title>
				<path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z" />
			</svg>
			{stars !== null && (
				<span className="flex items-center gap-1">
					<svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
						<title>Star</title>
						<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
					</svg>
					{stars}
				</span>
			)}
		</a>
	);
}
