"use client";

import * as React from "react";
import * as ToastPrimitive from "@radix-ui/react-toast";
import { X } from "lucide-react";
import { cn } from "../../lib/utils";

const ToastProvider = ToastPrimitive.Provider;

const ToastViewport = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Viewport>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Viewport>
>(({ className, ...props }, ref) => (
	<ToastPrimitive.Viewport
		ref={ref}
		className={cn(
			"fixed bottom-8 right-3 z-[100] flex max-h-screen w-full max-w-sm flex-col-reverse gap-2",
			className,
		)}
		{...props}
	/>
));
ToastViewport.displayName = ToastPrimitive.Viewport.displayName;

const Toast = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Root>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Root> & {
		variant?: "default" | "success" | "error";
	}
>(({ className, variant = "default", ...props }, ref) => (
	<ToastPrimitive.Root
		ref={ref}
		className={cn(
			"group pointer-events-auto relative flex w-full items-center gap-3 overflow-hidden rounded-md border p-3 font-mono shadow-lg transition-all",
			"data-[state=open]:animate-in data-[state=open]:slide-in-from-bottom-full data-[state=open]:fade-in-0",
			"data-[state=closed]:animate-out data-[state=closed]:fade-out-80 data-[state=closed]:slide-out-to-right-full",
			"data-[swipe=cancel]:translate-x-0 data-[swipe=end]:translate-x-[var(--radix-toast-swipe-end-x)] data-[swipe=move]:translate-x-[var(--radix-toast-swipe-move-x)] data-[swipe=move]:transition-none",
			{
				"border-border-light bg-surface text-text": variant === "default",
				"border-success/20 bg-success/5 text-success": variant === "success",
				"border-error/20 bg-error/5 text-error": variant === "error",
			},
			className,
		)}
		{...props}
	/>
));
Toast.displayName = ToastPrimitive.Root.displayName;

const ToastAction = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Action>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Action>
>(({ className, ...props }, ref) => (
	<ToastPrimitive.Action
		ref={ref}
		className={cn(
			"inline-flex shrink-0 items-center justify-center rounded-md border border-border-light bg-btn px-2.5 py-1 text-xs text-text-bright transition-colors",
			"hover:bg-btn-hover hover:border-border-hover",
			className,
		)}
		{...props}
	/>
));
ToastAction.displayName = ToastPrimitive.Action.displayName;

const ToastClose = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Close>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Close>
>(({ className, ...props }, ref) => (
	<ToastPrimitive.Close
		ref={ref}
		className={cn(
			"absolute right-2 top-2 rounded-sm text-text-muted opacity-0 transition-opacity",
			"hover:text-text-bright group-hover:opacity-100",
			className,
		)}
		toast-close=""
		{...props}
	>
		<X size={14} />
	</ToastPrimitive.Close>
));
ToastClose.displayName = ToastPrimitive.Close.displayName;

const ToastTitle = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Title>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Title>
>(({ className, ...props }, ref) => (
	<ToastPrimitive.Title
		ref={ref}
		className={cn("text-xs font-medium text-text-bright", className)}
		{...props}
	/>
));
ToastTitle.displayName = ToastPrimitive.Title.displayName;

const ToastDescription = React.forwardRef<
	React.ComponentRef<typeof ToastPrimitive.Description>,
	React.ComponentPropsWithoutRef<typeof ToastPrimitive.Description>
>(({ className, ...props }, ref) => (
	<ToastPrimitive.Description
		ref={ref}
		className={cn("text-xs text-text-muted", className)}
		{...props}
	/>
));
ToastDescription.displayName = ToastPrimitive.Description.displayName;

export {
	ToastProvider,
	ToastViewport,
	Toast,
	ToastTitle,
	ToastDescription,
	ToastClose,
	ToastAction,
};
