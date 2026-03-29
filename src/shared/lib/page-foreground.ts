import { useEffect, useState } from "react";

type FocusDocumentLike = {
	hasFocus?: () => boolean;
	visibilityState?: string;
};

export function isPageForeground(documentLike?: FocusDocumentLike | null) {
	if (!documentLike) {
		return true;
	}

	const visibilityState = documentLike.visibilityState ?? "visible";
	const hasFocus =
		typeof documentLike.hasFocus === "function"
			? documentLike.hasFocus()
			: true;
	return visibilityState === "visible" && hasFocus;
}

export function resolveForegroundPollInterval({
	active = true,
	activeMs,
	enabled = true,
	hiddenMs = false,
	inactiveMs = false,
	isForeground,
}: {
	active?: boolean;
	activeMs: number;
	enabled?: boolean;
	hiddenMs?: number | false;
	inactiveMs?: number | false;
	isForeground: boolean;
}): number | false {
	if (!enabled) {
		return false;
	}
	if (!isForeground) {
		return hiddenMs;
	}
	return active ? activeMs : inactiveMs;
}

export function usePageIsForeground() {
	const [isForeground, setIsForeground] = useState(() =>
		typeof document === "undefined" ? true : isPageForeground(document),
	);

	useEffect(() => {
		if (typeof window === "undefined") {
			return;
		}

		const update = () => {
			setIsForeground(isPageForeground(document));
		};

		update();
		window.addEventListener("focus", update);
		window.addEventListener("blur", update);
		document.addEventListener("visibilitychange", update);
		return () => {
			window.removeEventListener("focus", update);
			window.removeEventListener("blur", update);
			document.removeEventListener("visibilitychange", update);
		};
	}, []);

	return isForeground;
}
