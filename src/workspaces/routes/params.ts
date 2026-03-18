import { useParams } from "react-router-dom";

export function useWorkspaceRouteParams() {
	const params = useParams();

	return {
		project: params.project ?? "",
		workspaceName: params.workspace ?? "",
	};
}

export function useWorkspaceSessionRouteParams() {
	const params = useParams();

	return {
		project: params.project ?? "",
		workspaceName: params.workspace ?? "",
		attachmentId: params.attachmentId ?? "",
	};
}
