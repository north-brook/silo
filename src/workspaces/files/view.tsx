import { cpp } from "@codemirror/lang-cpp";
import { css } from "@codemirror/lang-css";
import { go } from "@codemirror/lang-go";
import { html } from "@codemirror/lang-html";
import { java } from "@codemirror/lang-java";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { php } from "@codemirror/lang-php";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import {
	type QueryClient,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import CodeMirror from "@uiw/react-codemirror";
import { AlertTriangle, Clock3, FileCode2, RefreshCw } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { useNavigate } from "react-router-dom";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import {
	type FileReadResult,
	filesRead,
	filesSave,
} from "@/workspaces/files/api";
import { useFileSessions } from "@/workspaces/files/context";
import { useWorkspaceSessionRouteParams } from "@/workspaces/routes/params";
import { fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

const editorTheme = EditorView.theme({
	"&": {
		height: "100%",
		backgroundColor: "var(--color-surface)",
		color: "var(--color-text-bright)",
		fontSize: "12px",
	},
	".cm-editor": {
		height: "100%",
		backgroundColor: "var(--color-surface)",
	},
	".cm-scroller": {
		height: "100%",
		fontFamily: "var(--font-mono)",
		overflow: "auto",
		padding: "12px 0",
		backgroundColor: "var(--color-surface)",
	},
	".cm-content": {
		minHeight: "100%",
		caretColor: "var(--color-text-bright)",
	},
	".cm-gutters": {
		backgroundColor: "var(--color-surface)",
		borderRight: "1px solid var(--color-border-light)",
		color: "var(--color-text-muted)",
	},
	".cm-activeLine": {
		backgroundColor: "rgba(99, 140, 255, 0.06)",
	},
	".cm-activeLineGutter": {
		backgroundColor: "rgba(99, 140, 255, 0.08)",
	},
	".cm-selectionBackground, .cm-content ::selection": {
		backgroundColor: "rgba(99, 140, 255, 0.22)",
	},
	".cm-cursor, .cm-dropCursor": {
		borderLeftColor: "var(--color-text-bright)",
	},
});

export function WorkspaceFileSessionView() {
	const navigate = useNavigate();
	const queryClient = useQueryClient();
	const sessions = useWorkspaceSessions();
	const {
		attachmentId,
		project,
		workspaceName: workspace,
	} = useWorkspaceSessionRouteParams();
	const {
		getWatchedFileState,
		promotePreviewTab,
		resolveSession,
		setSessionState,
	} = useFileSessions();
	const session = resolveSession(sessions, attachmentId);
	const path = session?.path ?? null;
	const watchedFile = getWatchedFileState(path);
	const promotingRef = useRef(false);
	const [loadedPath, setLoadedPath] = useState("");
	const [savedContent, setSavedContent] = useState("");
	const [buffer, setBuffer] = useState("");
	const [baseRevision, setBaseRevision] = useState("");
	const [conflict, setConflict] = useState<{
		content: string | null;
		revision: string;
	} | null>(null);

	const fileQuery = useQuery({
		queryKey: ["files_read", workspace, path],
		queryFn: () => filesRead(workspace, path ?? ""),
		enabled: !!workspace && !!path,
	});

	const dirty = buffer !== savedContent;

	useEffect(() => {
		if (!path) {
			return;
		}
		if (loadedPath && loadedPath !== path) {
			setLoadedPath("");
			setSavedContent("");
			setBuffer("");
			setBaseRevision("");
			setConflict(null);
		}
	}, [loadedPath, path]);

	useEffect(() => {
		const next = fileQuery.data;
		if (!next || !path) {
			return;
		}

		if (loadedPath !== path) {
			applyRemoteFile(next, {
				setBaseRevision,
				setBuffer,
				setConflict,
				setLoadedPath,
				setSavedContent,
			});
			return;
		}

		if (next.revision === baseRevision) {
			return;
		}

		if (!dirty) {
			applyRemoteFile(next, {
				setBaseRevision,
				setBuffer,
				setConflict,
				setLoadedPath,
				setSavedContent,
			});
			return;
		}

		setConflict({
			content: next.content,
			revision: next.revision,
		});
	}, [baseRevision, dirty, fileQuery.data, loadedPath, path]);

	useEffect(() => {
		if (!path || !watchedFile || fileQuery.isFetching) {
			return;
		}
		if (loadedPath !== path && !fileQuery.data) {
			void fileQuery.refetch();
			return;
		}
		if (watchedFile.revision === baseRevision) {
			return;
		}
		void fileQuery.refetch();
	}, [baseRevision, fileQuery, loadedPath, path, watchedFile]);

	const saveMutation = useMutation({
		mutationFn: async ({
			content,
			overrideRevision,
		}: {
			content: string;
			overrideRevision?: string;
		}) =>
			filesSave(
				workspace,
				path ?? "",
				content,
				overrideRevision ?? baseRevision,
			),
		onMutate: () => {
			setSessionState(attachmentId, { saving: true });
		},
		onSuccess: async (result, variables) => {
			if (result.status === "saved") {
				setSavedContent(variables.content);
				setBuffer(variables.content);
				setBaseRevision(result.revision ?? baseRevision);
				setConflict(null);
				setSessionState(attachmentId, {
					conflicted: false,
					dirty: false,
					saving: false,
				});
				await invalidateFileQueries(queryClient, workspace, path);
				return;
			}

			setSessionState(attachmentId, { saving: false });
			await invalidateFileQueries(queryClient, workspace, path);
			if (result.status === "missing") {
				toast({
					variant: "error",
					title: "File is gone",
					description: "The file no longer exists in the workspace.",
				});
				return;
			}

			toast({
				variant: "error",
				title: "File changed remotely",
				description:
					"The workspace copy changed while you were editing. Reload or overwrite it.",
			});
		},
		onError: (error) => {
			setSessionState(attachmentId, { saving: false });
			toast({
				variant: "error",
				title: "Failed to save file",
				description: error.message,
			});
		},
	});

	useEffect(() => {
		setSessionState(attachmentId, {
			conflicted: !!conflict,
			dirty,
		});
	}, [attachmentId, conflict, dirty, setSessionState]);

	useEffect(() => {
		setSessionState(attachmentId, { saving: saveMutation.isPending });
	}, [attachmentId, saveMutation.isPending, setSessionState]);

	const promotePreview = useCallback(async () => {
		if (!session?.preview || !workspace || promotingRef.current) {
			return;
		}
		promotingRef.current = true;
		try {
			const nextAttachmentId = await promotePreviewTab(
				workspace,
				sessions,
				attachmentId,
			);
			if (!nextAttachmentId || nextAttachmentId === attachmentId) {
				return;
			}
			navigate(
				fileSessionHref({
					project,
					workspace,
					attachmentId: nextAttachmentId,
				}),
				{ replace: true },
			);
		} finally {
			promotingRef.current = false;
		}
	}, [
		attachmentId,
		navigate,
		project,
		promotePreviewTab,
		session?.preview,
		sessions,
		workspace,
	]);

	useEffect(() => {
		const handler = (event: KeyboardEvent) => {
			if (!path) {
				return;
			}
			if (!event.metaKey || event.key.toLowerCase() !== "s") {
				return;
			}
			event.preventDefault();
			if (!dirty || saveMutation.isPending || conflict?.revision) {
				return;
			}
			void saveMutation.mutateAsync({ content: buffer });
		};
		window.addEventListener("keydown", handler);
		return () => {
			window.removeEventListener("keydown", handler);
		};
	}, [buffer, conflict?.revision, dirty, path, saveMutation]);

	if (!session || !path) {
		return null;
	}

	const showLoadingState = fileQuery.isLoading && loadedPath !== path;
	const remoteFile = fileQuery.data;
	const missing = remoteFile?.exists === false;
	const binary = remoteFile?.binary === true;

	return (
		<div className="flex-1 min-h-0 bg-surface flex flex-col overflow-hidden">
			{conflict && (
				<div className="shrink-0 border-b border-red-500/20 bg-red-500/6 px-4 py-2.5 flex items-center justify-between gap-4 text-[11px]">
					<div className="min-w-0 flex items-center gap-2 text-red-200">
						<AlertTriangle size={12} className="shrink-0" />
						<span className="truncate">
							The workspace version changed while this tab was dirty.
						</span>
					</div>
					<div className="shrink-0 flex items-center gap-2">
						<button
							type="button"
							onClick={() => {
								if (!remoteFile) return;
								applyRemoteFile(remoteFile, {
									setBaseRevision,
									setBuffer,
									setConflict,
									setLoadedPath,
									setSavedContent,
								});
							}}
							className="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-btn text-text hover:bg-btn-hover transition-colors"
						>
							<RefreshCw size={11} />
							Reload
						</button>
						<button
							type="button"
							disabled={!remoteFile?.exists || remoteFile.binary}
							onClick={() =>
								void saveMutation.mutateAsync({
									content: buffer,
									overrideRevision: conflict.revision,
								})
							}
							className="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-red-600 text-white hover:bg-red-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
						>
							<Clock3 size={11} />
							Overwrite
						</button>
					</div>
				</div>
			)}
			<div
				className="flex-1 min-h-0 overflow-hidden bg-surface"
				onFocusCapture={() => {
					void promotePreview();
				}}
				onMouseDownCapture={() => {
					void promotePreview();
				}}
			>
				{showLoadingState ? (
					<div className="h-full flex items-center justify-center">
						<Loader />
					</div>
				) : missing ? (
					<PlaceholderState
						icon={<AlertTriangle size={18} className="text-red-300" />}
						title="File missing"
						description="This file no longer exists in the workspace."
					/>
				) : binary ? (
					<PlaceholderState
						icon={<FileCode2 size={18} className="text-text-muted" />}
						title="Binary file"
						description="Binary files open in a read-only placeholder in this first pass."
					/>
				) : (
					<CodeMirror
						value={buffer}
						height="100%"
						extensions={[editorTheme, languageExtensionForPath(path)]}
						basicSetup={{
							foldGutter: false,
							highlightActiveLine: true,
							highlightActiveLineGutter: true,
						}}
						onChange={(value) => {
							if (session.preview) {
								void promotePreview();
							}
							setBuffer(value);
						}}
					/>
				)}
			</div>
		</div>
	);
}

function PlaceholderState({
	description,
	icon,
	title,
}: {
	description: string;
	icon: ReactNode;
	title: string;
}) {
	return (
		<div className="h-full flex flex-col items-center justify-center gap-3 text-center px-6">
			<div className="w-10 h-10 rounded-xl border border-border-light bg-btn flex items-center justify-center">
				{icon}
			</div>
			<div className="space-y-1">
				<div className="text-[12px] text-text-bright">{title}</div>
				<div className="text-[11px] text-text-muted max-w-sm">
					{description}
				</div>
			</div>
		</div>
	);
}

async function invalidateFileQueries(
	queryClient: QueryClient,
	workspace: string,
	path: string | null,
) {
	await Promise.all([
		path
			? queryClient.invalidateQueries({
					queryKey: ["files_read", workspace, path],
				})
			: Promise.resolve(),
		queryClient.invalidateQueries({
			queryKey: ["files_get_watched_state", workspace],
		}),
		queryClient.invalidateQueries({ queryKey: ["files_list_tree", workspace] }),
		queryClient.invalidateQueries({ queryKey: ["git_diff", workspace] }),
	]);
}

function applyRemoteFile(
	file: FileReadResult,
	setters: {
		setBaseRevision: (value: string) => void;
		setBuffer: (value: string) => void;
		setConflict: (
			value: {
				content: string | null;
				revision: string;
			} | null,
		) => void;
		setLoadedPath: (value: string) => void;
		setSavedContent: (value: string) => void;
	},
) {
	const nextContent = file.content ?? "";
	setters.setLoadedPath(file.path);
	setters.setSavedContent(nextContent);
	setters.setBuffer(nextContent);
	setters.setBaseRevision(file.revision);
	setters.setConflict(null);
}

function languageExtensionForPath(path: string): Extension {
	const name = path.split("/").slice(-1)[0]?.toLowerCase() ?? "";
	const extension = name.includes(".")
		? (name.split(".").slice(-1)[0] ?? "")
		: "";

	if (["ts", "tsx"].includes(extension)) {
		return javascript({ jsx: extension === "tsx", typescript: true });
	}
	if (["js", "jsx", "mjs", "cjs"].includes(extension)) {
		return javascript({ jsx: extension === "jsx" });
	}
	if (extension === "json") return json();
	if (["md", "mdx"].includes(extension)) return markdown();
	if (["html", "htm"].includes(extension)) return html();
	if (["css", "scss"].includes(extension)) return css();
	if (extension === "py") return python();
	if (extension === "rs") return rust();
	if (["yaml", "yml"].includes(extension)) return yaml();
	if (["xml", "svg"].includes(extension)) return xml();
	if (extension === "sql") return sql();
	if (["c", "cc", "cpp", "cxx", "h", "hpp"].includes(extension)) return cpp();
	if (extension === "java") return java();
	if (extension === "php") return php();
	if (extension === "go") return go();

	return [];
}
