"use client";

import * as React from "react";
import * as ToastPrimitive from "@radix-ui/react-toast";
import { X } from "lucide-react";
import { cn } from "../../lib/utils";

const TOAST_LIMIT = 3;
const TOAST_REMOVE_DELAY = 350;

type Variant = "default" | "success" | "error";

interface ToastData {
	id: string;
	variant?: Variant;
	title?: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}

type Action =
	| { type: "ADD_TOAST"; toast: ToastData }
	| { type: "UPDATE_TOAST"; toast: Partial<ToastData> }
	| { type: "DISMISS_TOAST"; toastId?: string }
	| { type: "REMOVE_TOAST"; toastId?: string };

interface State {
	toasts: ToastData[];
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
	variant?: Variant;
	title?: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
}

export function toast(props: ToastInput) {
	const id = genId();
	const dismiss = () => dispatch({ type: "DISMISS_TOAST", toastId: id });

	dispatch({
		type: "ADD_TOAST",
		toast: {
			...props,
			id,
			open: true,
			onOpenChange: (open) => {
				if (!open) dismiss();
			},
		},
	});

	return { id, dismiss };
}

function useToastState() {
	const [state, setState] = React.useState<State>(memoryState);

	React.useEffect(() => {
		listeners.push(setState);
		return () => {
			const index = listeners.indexOf(setState);
			if (index > -1) listeners.splice(index, 1);
		};
	}, []);

	return state;
}

const variantStyles: Record<Variant, string> = {
	default: "border-border-light bg-surface text-text",
	success: "border-success/20 bg-success/5 text-success",
	error: "border-error/20 bg-error/5 text-error",
};

export function Toaster() {
	const { toasts } = useToastState();

	return (
		<ToastPrimitive.Provider>
			{toasts.map(({ id, title, description, action, variant = "default", ...props }, index) => (
				<ToastPrimitive.Root
					key={id}
					duration={2000}
					className={cn(
						"group pointer-events-auto absolute bottom-0 right-0 flex w-full items-center gap-3 overflow-hidden rounded-lg border p-3 font-mono shadow-lg",
						"data-[state=open]:toast-enter data-[state=closed]:toast-exit",
						"data-[swipe=cancel]:translate-x-0 data-[swipe=end]:translate-x-[var(--radix-toast-swipe-end-x)] data-[swipe=move]:translate-x-[var(--radix-toast-swipe-move-x)] data-[swipe=move]:transition-none",
						variantStyles[variant],
					)}
					style={{
						transition: "transform 400ms cubic-bezier(0.16, 1, 0.3, 1), opacity 400ms cubic-bezier(0.16, 1, 0.3, 1)",
						transform: `translateY(${-index * 10}px) scale(${1 - index * 0.04})`,
						opacity: 1 - index * 0.15,
						zIndex: toasts.length - index,
					}}
					{...props}
				>
					<div className="flex flex-col gap-1">
						{title && (
							<ToastPrimitive.Title className="text-xs font-medium text-text-bright">
								{title}
							</ToastPrimitive.Title>
						)}
						{description && (
							<ToastPrimitive.Description className="text-xs text-text-muted">
								{description}
							</ToastPrimitive.Description>
						)}
					</div>
					{action}
					<ToastPrimitive.Close className="absolute right-2 top-2 rounded-sm text-text-muted opacity-0 transition-opacity hover:text-text-bright group-hover:opacity-100">
						<X size={14} />
					</ToastPrimitive.Close>
				</ToastPrimitive.Root>
			))}
			<ToastPrimitive.Viewport className="fixed bottom-3 right-3 z-[100] flex max-h-screen w-64" />
		</ToastPrimitive.Provider>
	);
}
