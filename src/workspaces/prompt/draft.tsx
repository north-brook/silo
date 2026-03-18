import {
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";

export type PromptProviderId = "codex" | "claude";

interface PromptDraft {
	prompt: string;
	providerId: PromptProviderId;
}

interface PromptContextValue {
	getDraft: (workspace: string) => PromptDraft;
	setPrompt: (workspace: string, prompt: string) => void;
	setProviderId: (workspace: string, providerId: PromptProviderId) => void;
	clearDraft: (workspace: string) => void;
}

const DEFAULT_DRAFT: PromptDraft = {
	prompt: "",
	providerId: "codex",
};

const PromptContext = createContext<PromptContextValue | null>(null);

export function PromptDraftProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [draftsByWorkspace, setDraftsByWorkspace] = useState<
		Record<string, PromptDraft>
	>({});

	const getDraft = useCallback(
		(workspace: string) => draftsByWorkspace[workspace] ?? DEFAULT_DRAFT,
		[draftsByWorkspace],
	);

	const setPrompt = useCallback((workspace: string, prompt: string) => {
		setDraftsByWorkspace((current) => {
			const existing = current[workspace] ?? DEFAULT_DRAFT;
			if (existing.prompt === prompt) {
				return current;
			}
			return {
				...current,
				[workspace]: {
					...existing,
					prompt,
				},
			};
		});
	}, []);

	const setProviderId = useCallback(
		(workspace: string, providerId: PromptProviderId) => {
			setDraftsByWorkspace((current) => {
				const existing = current[workspace] ?? DEFAULT_DRAFT;
				if (existing.providerId === providerId) {
					return current;
				}
				return {
					...current,
					[workspace]: {
						...existing,
						providerId,
					},
				};
			});
		},
		[],
	);

	const clearDraft = useCallback((workspace: string) => {
		setDraftsByWorkspace((current) => {
			if (!(workspace in current)) {
				return current;
			}
			const next = { ...current };
			delete next[workspace];
			return next;
		});
	}, []);

	const value = useMemo<PromptContextValue>(
		() => ({
			getDraft,
			setPrompt,
			setProviderId,
			clearDraft,
		}),
		[getDraft, setPrompt, setProviderId, clearDraft],
	);

	return (
		<PromptContext.Provider value={value}>{children}</PromptContext.Provider>
	);
}

export function usePromptDraft(workspace: string) {
	const context = useContext(PromptContext);
	if (!context) {
		throw new Error("usePromptDraft must be used within a PromptDraftProvider");
	}

	const draft = context.getDraft(workspace);

	return {
		prompt: draft.prompt,
		providerId: draft.providerId,
		setPrompt: (prompt: string) => context.setPrompt(workspace, prompt),
		setProviderId: (providerId: PromptProviderId) =>
			context.setProviderId(workspace, providerId),
		clearDraft: () => context.clearDraft(workspace),
	};
}
