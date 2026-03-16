"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect } from "react";

export default function LegacyTerminalRoutePage() {
	return (
		<Suspense>
			<LegacyTerminalRouteView />
		</Suspense>
	);
}

function LegacyTerminalRouteView() {
	const router = useRouter();
	const searchParams = useSearchParams();

	useEffect(() => {
		const params = new URLSearchParams(searchParams.toString());
		if (!params.get("kind")) {
			params.set("kind", "terminal");
		}
		router.replace(`/workspace/session?${params.toString()}`);
	}, [router, searchParams]);

	return null;
}
