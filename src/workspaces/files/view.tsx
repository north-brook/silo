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
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import {
	type EditorState,
	type Extension,
	RangeSetBuilder,
	StateField,
} from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
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
	useMemo,
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
import { filePathOpensInBrowser } from "@/workspaces/files/browser";
import { useFileSessions } from "@/workspaces/files/context";
import { gitDiffFile } from "@/workspaces/git/api";
import { useWorkspaceSessionRouteParams } from "@/workspaces/routes/params";
import { fileSessionHref } from "@/workspaces/routes/paths";
import { useWorkspaceSessions } from "@/workspaces/state";

const editorTheme = EditorView.theme(
	{
		"&": {
			height: "100%",
			backgroundColor: "var(--color-surface)",
			color: "var(--color-text)",
			fontSize: "var(--text-base)",
		},
		".cm-scroller": {
			fontFamily: "var(--font-mono)",
			overflow: "auto",
		},
		".cm-scroller::-webkit-scrollbar": {
			width: "10px",
		},
		".cm-scroller::-webkit-scrollbar-track": {
			background: "transparent",
		},
		".cm-scroller::-webkit-scrollbar-thumb": {
			background: "var(--color-border-light)",
			borderRadius: "0",
			border: "2px solid transparent",
			backgroundClip: "padding-box",
		},
		".cm-scroller::-webkit-scrollbar-thumb:hover": {
			background: "var(--color-border-hover)",
			borderRadius: "0",
			border: "2px solid transparent",
			backgroundClip: "padding-box",
		},
		".cm-content": {
			caretColor: "var(--color-text-bright)",
		},
		".cm-gutters": {
			backgroundColor: "var(--color-surface)",
			borderRight: "none",
			color: "var(--color-text-placeholder)",
		},
		".cm-activeLineGutter": {
			backgroundColor: "transparent",
			color: "var(--color-text-muted)",
		},
		".cm-activeLine": {
			backgroundColor: "rgba(255, 255, 255, 0.02)",
		},
		".cm-selectionBackground, .cm-content ::selection": {
			backgroundColor: "rgba(99, 140, 255, 0.15) !important",
		},
		".cm-cursor, .cm-dropCursor": {
			borderLeftColor: "var(--color-text-bright)",
		},
		".cm-matchingBracket": {
			backgroundColor: "rgba(99, 140, 255, 0.12)",
			outline: "none",
		},
		".cm-diff-added": {
			backgroundColor: "rgba(52, 211, 153, 0.06)",
			boxShadow: "inset 2px 0 0 rgba(52, 211, 153, 0.5)",
		},
		".cm-diff-modified": {
			backgroundColor: "rgba(252, 211, 77, 0.06)",
			boxShadow: "inset 2px 0 0 rgba(252, 211, 77, 0.5)",
		},
	},
	{ dark: true },
);

const highlightStyle = HighlightStyle.define([
	{ tag: [t.keyword, t.operatorKeyword, t.controlKeyword], color: "#ff7b72" },
	{ tag: [t.moduleKeyword], color: "#ff7b72" },
	{ tag: [t.modifier], color: "#e6edf3" },
	{ tag: [t.string, t.special(t.string)], color: "#a5d6ff" },
	{ tag: [t.regexp], color: "#a5d6ff" },
	{
		tag: [t.typeName, t.className, t.namespace],
		color: "#ffa657",
	},
	{ tag: [t.tagName], color: "#7ee787" },
	{ tag: [t.number, t.bool, t.null, t.atom], color: "#79c0ff" },
	{ tag: [t.self], color: "#ff7b72" },
	{ tag: t.constant(t.name), color: "#79c0ff" },
	{
		tag: [t.function(t.variableName), t.function(t.propertyName)],
		color: "#d2a8ff",
	},
	{ tag: t.definition(t.variableName), color: "#e6edf3" },
	{ tag: [t.variableName], color: "#e6edf3" },
	{ tag: [t.propertyName], color: "#79c0ff" },
	{ tag: t.definition(t.propertyName), color: "#79c0ff" },
	{ tag: t.operator, color: "#ff7b72" },
	{ tag: [t.punctuation, t.separator, t.bracket], color: "#e6edf3" },
	{
		tag: [t.comment, t.lineComment, t.blockComment],
		color: "#8b949e",
		fontStyle: "italic",
	},
	{ tag: [t.meta, t.annotation], color: "#ff7b72" },
	{ tag: t.attributeName, color: "#79c0ff" },
	{ tag: t.attributeValue, color: "#a5d6ff" },
	{ tag: t.heading, fontWeight: "bold", color: "#79c0ff" },
	{ tag: t.strong, fontWeight: "bold" },
	{ tag: t.emphasis, fontStyle: "italic" },
	{ tag: t.link, color: "#79c0ff", textDecoration: "underline" },
	{ tag: t.url, color: "#79c0ff" },
	{ tag: t.inserted, color: "#7ee787" },
	{ tag: t.deleted, color: "#ffa198" },
	{ tag: t.changed, color: "#79c0ff" },
	{ tag: t.escape, color: "#79c0ff" },
	{ tag: t.invalid, color: "#ffa198" },
]);

// --- Diff line decorations (from git patch) ---

const addedLineDeco = Decoration.line({ class: "cm-diff-added" });
const modifiedLineDeco = Decoration.line({ class: "cm-diff-modified" });

function parsePatchChangedLines(
	patch: string,
): Map<number, "added" | "modified"> {
	const result = new Map<number, "added" | "modified">();
	const lines = patch.split("\n");

	let newLine = 0;
	let pendingDeletions = 0;
	let pendingAdditions: number[] = [];

	const flushBlock = () => {
		if (pendingAdditions.length === 0) {
			pendingDeletions = 0;
			return;
		}
		const modifiedCount = Math.min(pendingDeletions, pendingAdditions.length);
		for (let i = 0; i < pendingAdditions.length; i++) {
			result.set(pendingAdditions[i], i < modifiedCount ? "modified" : "added");
		}
		pendingDeletions = 0;
		pendingAdditions = [];
	};

	for (const line of lines) {
		if (line.startsWith("@@")) {
			const hunkMatch = line.match(/^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
			if (hunkMatch) {
				flushBlock();
				newLine = Number.parseInt(hunkMatch[1], 10);
			}
			continue;
		}

		if (newLine === 0) continue;

		if (line.startsWith("+")) {
			pendingAdditions.push(newLine);
			newLine++;
		} else if (line.startsWith("-")) {
			pendingDeletions++;
		} else if (line.startsWith("\\")) {
			// "\ No newline at end of file" marker
		} else if (line.startsWith(" ")) {
			flushBlock();
			newLine++;
		} else if (line === "") {
			// Empty context line (git may omit space prefix)
			flushBlock();
			newLine++;
		} else {
			// Non-hunk line (next file header, etc.)
			flushBlock();
			newLine = 0;
		}
	}

	flushBlock();
	return result;
}

function createPatchDiffExtension(
	changedLines: Map<number, "added" | "modified">,
): Extension {
	return StateField.define<DecorationSet>({
		create(state) {
			return buildPatchDecorations(changedLines, state);
		},
		update(value, tr) {
			if (!tr.docChanged) return value;
			return value.map(tr.changes);
		},
		provide(field) {
			return EditorView.decorations.from(field);
		},
	});
}

function buildPatchDecorations(
	changedLines: Map<number, "added" | "modified">,
	state: EditorState,
): DecorationSet {
	if (changedLines.size === 0) return Decoration.none;

	const builder = new RangeSetBuilder<Decoration>();
	const doc = state.doc;

	for (let i = 1; i <= doc.lines; i++) {
		const status = changedLines.get(i);
		if (status) {
			const line = doc.line(i);
			builder.add(
				line.from,
				line.from,
				status === "added" ? addedLineDeco : modifiedLineDeco,
			);
		}
	}

	return builder.finish();
}

// --- Component ---

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
	const diffFileQuery = useQuery({
		queryKey: ["git_diff", workspace, "file", path],
		queryFn: () => gitDiffFile(workspace, path ?? ""),
		enabled: !!workspace && !!path,
		gcTime: 60 * 1000,
	});

	const dirty = buffer !== savedContent;

	// Parse git diff patch for the current file to get changed line numbers
	const changedLines = useMemo(() => {
		if (!path || !diffFileQuery.data?.patch) return null;
		return parsePatchChangedLines(diffFileQuery.data.patch);
	}, [diffFileQuery.data?.patch, path]);

	const extensions = useMemo(
		() => [
			syntaxHighlighting(highlightStyle),
			EditorView.lineWrapping,
			...(changedLines && changedLines.size > 0
				? [createPatchDiffExtension(changedLines)]
				: []),
			languageExtensionForPath(path ?? ""),
		],
		[changedLines, path],
	);

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
				queryClient.setQueryData<FileReadResult>(
					["files_read", workspace, path],
					(current) => ({
						path: path ?? current?.path ?? "",
						exists: true,
						binary: false,
						revision: result.revision ?? baseRevision,
						content: variables.content,
					}),
				);
				setSavedContent(variables.content);
				setBuffer(variables.content);
				setBaseRevision(result.revision ?? baseRevision);
				setConflict(null);
				setSessionState(attachmentId, {
					conflicted: false,
					dirty: false,
					saving: false,
				});
				await invalidateFileQueries(queryClient, workspace, path, {
					includeRead: false,
				});
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
				<div className="shrink-0 border-b border-red-500/20 bg-red-500/6 px-4 py-2.5 flex items-center justify-between gap-4 text-sm">
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
				className="flex-1 min-h-0 relative overflow-hidden bg-surface"
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
						description={
							filePathOpensInBrowser(path)
								? "Open this file from the explorer to view it in a browser tab."
								: "Binary files open in a read-only placeholder in this first pass."
						}
					/>
				) : (
					<CodeMirror
						value={buffer}
						style={{
							position: "absolute",
							top: 0,
							left: 0,
							right: 0,
							bottom: 0,
						}}
						theme={editorTheme}
						extensions={extensions}
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
				<div className="text-sm text-text-bright">{title}</div>
				<div className="text-sm text-text-muted max-w-sm">
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
	options?: {
		includeRead?: boolean;
	},
) {
	await Promise.all([
		(options?.includeRead ?? true) && path
			? queryClient.invalidateQueries({
					queryKey: ["files_read", workspace, path],
				})
			: Promise.resolve(),
			queryClient.invalidateQueries({
				queryKey: ["files_get_watched_state", workspace],
			}),
			queryClient.invalidateQueries({
				queryKey: ["files_list_directory", workspace],
			}),
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
