import type { CloudSessionKind } from "@/workspaces/hosts/model";

export function cloudSessionHref({
	project,
	workspace,
	kind,
	attachmentId,
	fresh,
}: {
	project: string;
	workspace: string;
	kind: CloudSessionKind;
	attachmentId: string;
	fresh?: boolean;
}): string {
	const params = new URLSearchParams({
		project,
		workspace,
		kind,
		attachment_id: attachmentId,
	});

	if (fresh) {
		params.set("fresh", "1");
	}

	return `/workspace/session?${params.toString()}`;
}
