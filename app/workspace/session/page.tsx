"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect } from "react";

export default function WorkspaceSessionPage() {
	return (
		<Suspense>
			<WorkspaceSessionView />
		</Suspense>
	);
}

function WorkspaceSessionView() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const workspace = searchParams.get("workspace") ?? "";
	const attachmentId = searchParams.get("attachment_id") ?? "";
	const kind = searchParams.get("kind") ?? "";
	const fresh = searchParams.get("fresh") === "1";
	const cleanParams = new URLSearchParams(searchParams.toString());
	cleanParams.delete("fresh");
	const cleanUrl = `/workspace/session?${cleanParams.toString()}`;

	useEffect(() => {
		if (!fresh) {
			return;
		}
		router.replace(cleanUrl);
	}, [cleanUrl, fresh, router]);

	if (!workspace || !attachmentId || !kind) {
		return null;
	}

	if (kind === "terminal") {
		return null;
	}

	return (
		<div className="flex-1 min-h-0 bg-surface flex items-center justify-center p-6">
			<div className="text-[11px] text-text-muted">
				Unsupported session type: {kind}
			</div>
		</div>
	);
}
