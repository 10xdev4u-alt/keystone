import * as RadixToast from "@radix-ui/react-toast";
import { cn } from "../../lib/cn";
import { type ReactNode } from "react";
import "./toast.css";

export type ToastTone = "default" | "success" | "danger";

export interface ToastData {
  id: string;
  title: string;
  description?: string;
  tone?: ToastTone;
}

export interface ToastHostProps {
  toasts: ToastData[];
  onDismiss: (id: string) => void;
}

/**
 * Toast host — mount once at the app root, feed it from a store. Radix
 * handles the live region (polite) and focus restoration after dismiss.
 */
export function ToastHost({ toasts, onDismiss }: ToastHostProps) {
  return (
    <RadixToast.Provider swipeDirection="right">
      {toasts.map((toast) => (
        <RadixToast.Root
          key={toast.id}
          className={cn("toast", toast.tone && `toast--${toast.tone}`)}
          onOpenChange={(open) => {
            if (!open) onDismiss(toast.id);
          }}
        >
          <div className="toast__body">
            <RadixToast.Title className="toast__title">{toast.title}</RadixToast.Title>
            {toast.description && (
              <RadixToast.Description className="toast__description">
                {toast.description}
              </RadixToast.Description>
            )}
          </div>
          <RadixToast.Close aria-label="Dismiss notification">✕</RadixToast.Close>
        </RadixToast.Root>
      ))}
      <RadixToast.Viewport className="toast__viewport" />
    </RadixToast.Provider>
  );
}

export type { ReactNode };
