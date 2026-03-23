import * as ToastPrimitive from "@radix-ui/react-toast";
import { X } from "lucide-react";
import * as React from "react";
import { cn } from "../lib/utils";

const TOAST_LIMIT = 3;
const TOAST_REMOVE_DELAY = 350;

type Variant = "default" | "success" | "error";

interface ToastData {
	id: string;
	variant?: Variant;
	title?: React.ReactNode;
	description?: React.ReactNode;
	action?: React.ReactNode;
	duration?: number;
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
					t.id === toastId || toastId === undefined ? { ...t, open: false } : t,
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
	duration?: number;
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
	success: "border-[#1b2f24] bg-[#111a16] text-[#4ade80]",
	error: "border-[#2f1b20] bg-[#1a1114] text-[#f87171]",
};

export function Toaster() {
	const { toasts } = useToastState();

	return (
		<ToastPrimitive.Provider>
			{toasts.map(
				(
					{
						id,
						title,
						description,
						action,
						variant = "default",
						duration = 2000,
						...props
					},
					index,
				) => (
					<ToastPrimitive.Root
						key={id}
						duration={duration}
						className={cn(
							"group toast-root pointer-events-auto absolute bottom-0 right-0 flex w-full flex-col gap-2 overflow-hidden rounded-lg border p-3 font-mono shadow-lg",
							`toast-stack-${Math.min(index, TOAST_LIMIT - 1)}`,
							"data-[swipe=cancel]:translate-x-0 data-[swipe=end]:translate-x-[var(--radix-toast-swipe-end-x)] data-[swipe=move]:translate-x-[var(--radix-toast-swipe-move-x)] data-[swipe=move]:transition-none",
							variantStyles[variant],
						)}
						{...props}
					>
						<div className="flex flex-col gap-1">
							{title && (
								<ToastPrimitive.Title className="text-sm font-medium text-text">
									{title}
								</ToastPrimitive.Title>
							)}
							{description && (
								<ToastPrimitive.Description className="text-sm text-text-muted">
									{description}
								</ToastPrimitive.Description>
							)}
						</div>
						{action}
						<ToastPrimitive.Close className="absolute right-2 top-2 rounded-sm text-text-muted opacity-0 transition-opacity hover:text-text-bright group-hover:opacity-100">
							<X size={14} />
						</ToastPrimitive.Close>
					</ToastPrimitive.Root>
				),
			)}
			<ToastPrimitive.Viewport className="fixed bottom-3 left-3 z-[100] flex max-h-screen w-64" />
		</ToastPrimitive.Provider>
	);
}
