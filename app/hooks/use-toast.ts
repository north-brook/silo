"use client";

import * as React from "react";

import type { ComponentPropsWithoutRef } from "react";
import type { Toast } from "../components/toast";

const TOAST_LIMIT = 3;
const TOAST_REMOVE_DELAY = 1000;

type ToastProps = ComponentPropsWithoutRef<typeof Toast>;

type ToasterToast = Omit<ToastProps, "title"> & {
	id: string;
	title?: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
};

type Action =
	| { type: "ADD_TOAST"; toast: ToasterToast }
	| { type: "UPDATE_TOAST"; toast: Partial<ToasterToast> }
	| { type: "DISMISS_TOAST"; toastId?: string }
	| { type: "REMOVE_TOAST"; toastId?: string };

interface State {
	toasts: ToasterToast[];
}

const toastTimeouts = new Map<string, ReturnType<typeof setTimeout>>();

function addToRemoveQueue(toastId: string) {
	if (toastTimeouts.has(toastId)) return;

	const timeout = setTimeout(() => {
		toastTimeouts.delete(toastId);
		dispatch({ type: "REMOVE_TOAST", toastId });
	}, TOAST_REMOVE_DELAY);

	toastTimeouts.set(toastId, timeout);
}

let count = 0;
function genId() {
	count = (count + 1) % Number.MAX_SAFE_INTEGER;
	return count.toString();
}

function reducer(state: State, action: Action): State {
	switch (action.type) {
		case "ADD_TOAST":
			return {
				...state,
				toasts: [action.toast, ...state.toasts].slice(0, TOAST_LIMIT),
			};
		case "UPDATE_TOAST":
			return {
				...state,
				toasts: state.toasts.map((t) =>
					t.id === action.toast.id ? { ...t, ...action.toast } : t,
				),
			};
		case "DISMISS_TOAST": {
			const { toastId } = action;
			if (toastId) {
				addToRemoveQueue(toastId);
			} else {
				for (const t of state.toasts) {
					addToRemoveQueue(t.id);
				}
			}
			return {
				...state,
				toasts: state.toasts.map((t) =>
					t.id === toastId || toastId === undefined
						? { ...t, open: false }
						: t,
				),
			};
		}
		case "REMOVE_TOAST":
			if (action.toastId === undefined) return { ...state, toasts: [] };
			return {
				...state,
				toasts: state.toasts.filter((t) => t.id !== action.toastId),
			};
	}
}

const listeners: Array<(state: State) => void> = [];
let memoryState: State = { toasts: [] };

function dispatch(action: Action) {
	memoryState = reducer(memoryState, action);
	for (const listener of listeners) {
		listener(memoryState);
	}
}

interface ToastInput {
	variant?: ToastProps["variant"];
	title?: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
	duration?: number;
}

function toast({ ...props }: ToastInput) {
	const id = genId();

	const update = (updateProps: Partial<ToasterToast>) =>
		dispatch({ type: "UPDATE_TOAST", toast: { ...updateProps, id } });

	const dismiss = () => dispatch({ type: "DISMISS_TOAST", toastId: id });

	dispatch({
		type: "ADD_TOAST",
		toast: { ...props, id, open: true, onOpenChange: (open) => { if (!open) dismiss(); } },
	});

	return { id, dismiss, update };
}

function useToast() {
	const [state, setState] = React.useState<State>(memoryState);

	React.useEffect(() => {
		listeners.push(setState);
		return () => {
			const index = listeners.indexOf(setState);
			if (index > -1) listeners.splice(index, 1);
		};
	}, []);

	return {
		...state,
		toast,
		dismiss: (toastId?: string) =>
			dispatch({ type: "DISMISS_TOAST", toastId }),
	};
}

export { useToast, toast };
