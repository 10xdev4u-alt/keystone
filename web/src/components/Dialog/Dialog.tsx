import * as RadixDialog from "@radix-ui/react-dialog";
import { cn } from "../../lib/cn";
import { type ReactNode } from "react";
import "./dialog.css";

export interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  /** Footer actions (buttons). */
  footer?: ReactNode;
}

/**
 * Modal dialog with focus trap, Esc-to-close, scroll lock and title
 * announcements — all from Radix. `aria-describedby` is only wired when a
 * description exists (axe would flag an empty description).
 */
export function Dialog({ open, onOpenChange, title, description, children, footer }: DialogProps) {
  return (
    <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="dialog__overlay" />
        <RadixDialog.Content className="dialog__content">
          <RadixDialog.Title className="dialog__title">{title}</RadixDialog.Title>
          {description && (
            <RadixDialog.Description className="dialog__description">
              {description}
            </RadixDialog.Description>
          )}
          <div className="dialog__body">{children}</div>
          {footer && <div className={cn("dialog__footer")}>{footer}</div>}
          <RadixDialog.Close asChild>
            <button type="button" className="dialog__close" aria-label="Close dialog">
              <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
                <path
                  d="M2 2l10 10M12 2L2 12"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                />
              </svg>
            </button>
          </RadixDialog.Close>
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
