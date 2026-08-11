import { cn } from "../../lib/cn";
import { forwardRef, useId, type InputHTMLAttributes } from "react";
import "./input.css";

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  /** Renders a persistent error message wired to aria-invalid/aria-describedby. */
  error?: string;
  hint?: string;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ label, error, hint, id, className, ...props }, ref) => {
    const autoId = useId();
    const inputId = id ?? autoId;
    const errorId = `${inputId}-error`;
    const hintId = `${inputId}-hint`;

    return (
      <div className={cn("field", className)}>
        {label && (
          <label className="field__label" htmlFor={inputId}>
            {label}
          </label>
        )}
        <input
          ref={ref}
          id={inputId}
          className={cn("field__input", error && "field__input--invalid")}
          aria-invalid={error ? true : undefined}
          aria-describedby={
            [error ? errorId : null, hint && !error ? hintId : null]
              .filter(Boolean)
              .join(" ") || undefined
          }
          {...props}
        />
        {error ? (
          <p id={errorId} className="field__error" role="alert">
            {error}
          </p>
        ) : hint ? (
          <p id={hintId} className="field__hint">
            {hint}
          </p>
        ) : null}
      </div>
    );
  },
);
Input.displayName = "Input";
