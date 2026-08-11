import { Slot } from "@radix-ui/react-slot";
import { cn } from "../../lib/cn";
import { forwardRef, type ButtonHTMLAttributes } from "react";
import "./button.css";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Visual intent. Default: primary. */
  variant?: ButtonVariant;
  /** Control height. Default: md. */
  size?: ButtonSize;
  /** Render the button as a child element (link, router Link) while keeping
   *  styling, focus and disabled semantics. */
  asChild?: boolean;
  /** Replaces the label with a spinner and disables the button. */
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    { className, variant = "primary", size = "md", asChild, loading, disabled, children, ...props },
    ref,
  ) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        ref={ref}
        className={cn("btn", `btn--${variant}`, `btn--${size}`, className)}
        data-loading={loading || undefined}
        disabled={disabled || loading}
        aria-busy={loading || undefined}
        {...props}
      >
        {loading && (
          <span className="btn__spinner" aria-hidden="true" data-testid="button-spinner" />
        )}
        {children}
      </Comp>
    );
  },
);
Button.displayName = "Button";
