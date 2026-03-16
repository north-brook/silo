"use client";

import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";

interface OverlayStateValue {
	overlayOpen: boolean;
	setOverlayOpen: (id: string, open: boolean) => void;
}

const OverlayStateContext = createContext<OverlayStateValue | null>(null);

export function OverlayStateProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [openOverlays, setOpenOverlays] = useState<Record<string, boolean>>({});

	const setOverlayOpen = useCallback((id: string, open: boolean) => {
		setOpenOverlays((previous) => {
			const currentlyOpen = previous[id] === true;
			if (open === currentlyOpen) {
				return previous;
			}

			if (open) {
				return {
					...previous,
					[id]: true,
				};
			}

			const next = { ...previous };
			delete next[id];
			return next;
		});
	}, []);

	const value = useMemo<OverlayStateValue>(
		() => ({
			overlayOpen: Object.keys(openOverlays).length > 0,
			setOverlayOpen,
		}),
		[openOverlays, setOverlayOpen],
	);

	return (
		<OverlayStateContext.Provider value={value}>
			{children}
		</OverlayStateContext.Provider>
	);
}

export function useOverlayOpen() {
	const context = useContext(OverlayStateContext);
	if (!context) {
		throw new Error("useOverlayOpen must be used within an OverlayStateProvider");
	}
	return context.overlayOpen;
}

export function useOverlayRegistration(open: boolean) {
	const context = useContext(OverlayStateContext);
	if (!context) {
		throw new Error(
			"useOverlayRegistration must be used within an OverlayStateProvider",
		);
	}
	const id = useId();
	const setOverlayOpen = context.setOverlayOpen;

	useEffect(() => {
		setOverlayOpen(id, open);
		return () => {
			setOverlayOpen(id, false);
		};
	}, [id, open, setOverlayOpen]);
}
