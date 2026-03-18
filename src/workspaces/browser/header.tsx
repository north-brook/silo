import { useMutation } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, RotateCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { invoke } from "@/shared/lib/invoke";
import { shortcutEvents } from "@/shared/lib/shortcuts";
import { useShortcut } from "@/shared/lib/use-shortcut";
import { Loader } from "@/shared/ui/loader";
import { toast } from "@/shared/ui/toaster";
import type { CloudSession } from "@/workspaces/hosts/model";

export function BrowserSessionHeader({
	session,
	autoFocusAddress,
	onChanged,
}: {
	session: CloudSession;
	autoFocusAddress: boolean;
	onChanged: () => void;
}) {
	const inputRef = useRef<HTMLInputElement>(null);
	const [addressDraft, setAddressDraft] = useState(session.url ?? "");
	const [isEditingAddress, setIsEditingAddress] = useState(autoFocusAddress);

	useEffect(() => {
		if (autoFocusAddress) {
			inputRef.current?.focus();
		}
	}, [autoFocusAddress]);

	const navigate = useMutation({
		mutationFn: (url: string) =>
			invoke("browser_go_to", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
				url,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to navigate",
				description: error.message,
			});
		},
	});
	const address = isEditingAddress
		? addressDraft
		: navigate.isPending
			? (navigate.variables ?? session.url ?? "")
			: (session.url ?? "");

	const goBack = useMutation({
		mutationFn: () =>
			invoke("browser_go_back", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to go back",
				description: error.message,
			});
		},
	});

	useShortcut<void>({
		event: shortcutEvents.goBackBrowser,
		onTrigger: () => {
			if (session.canGoBack !== false && !goBack.isPending) {
				goBack.mutate();
			}
		},
	});

	const goForward = useMutation({
		mutationFn: () =>
			invoke("browser_go_forward", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to go forward",
				description: error.message,
			});
		},
	});

	useShortcut<void>({
		event: shortcutEvents.goForwardBrowser,
		onTrigger: () => {
			if (session.canGoForward !== false && !goForward.isPending) {
				goForward.mutate();
			}
		},
	});

	const refresh = useMutation({
		mutationFn: () =>
			invoke("browser_refresh_page", {
				workspace: session.workspace,
				attachmentId: session.attachmentId,
			}),
		onSuccess: onChanged,
		onError: (error) => {
			toast({
				variant: "error",
				title: "Failed to refresh",
				description: error.message,
			});
		},
	});

	useShortcut<void>({
		event: shortcutEvents.refreshBrowser,
		onTrigger: () => {
			if (!refresh.isPending) {
				refresh.mutate();
			}
		},
	});

	const busy =
		navigate.isPending ||
		goBack.isPending ||
		goForward.isPending ||
		refresh.isPending;

	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				setIsEditingAddress(false);
				navigate.mutate(address);
			}}
			className="h-9 shrink-0 bg-surface px-1.5 flex items-center gap-0.5"
		>
			<button
				type="button"
				disabled={busy || session.canGoBack === false}
				onClick={() => goBack.mutate()}
				aria-label="Back"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				<ArrowLeft size={12} />
			</button>
			<button
				type="button"
				disabled={busy || session.canGoForward === false}
				onClick={() => goForward.mutate()}
				aria-label="Forward"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				<ArrowRight size={12} />
			</button>
			<button
				type="button"
				disabled={busy}
				onClick={() => refresh.mutate()}
				aria-label="Refresh"
				className="h-7 w-7 rounded-md flex items-center justify-center text-text-muted hover:text-text-bright hover:bg-btn-hover disabled:opacity-40 disabled:hover:bg-transparent transition-colors"
			>
				{busy ? <Loader className="text-text-muted" /> : <RotateCw size={12} />}
			</button>
			<input
				ref={inputRef}
				value={address}
				onBlur={() => {
					setIsEditingAddress(false);
					setAddressDraft(session.url ?? "");
				}}
				onChange={(event) => {
					setAddressDraft(event.target.value);
				}}
				onFocus={() => {
					setAddressDraft(session.url ?? "");
					setIsEditingAddress(true);
				}}
				placeholder="Enter URL"
				spellCheck={false}
				autoCorrect="off"
				autoCapitalize="off"
				className="flex-1 min-w-0 h-7 rounded-md bg-bg px-2.5 text-[12px] text-text-bright outline-none border border-border-light focus:border-text-muted transition-colors"
			/>
		</form>
	);
}
