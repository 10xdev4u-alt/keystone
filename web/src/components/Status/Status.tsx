import { cn } from "../../lib/cn";
import { type ReactNode } from "react";
import "./status.css";

/** Loading placeholder — shimmers while `aria-hidden` to keep the a11y tree clean. */
export function Skeleton({ className }: { className?: string }) {
  return <div className={cn("skeleton", className)} aria-hidden="true" data-testid="skeleton" />;
}

export interface EmptyStateProps {
  title: string;
  description?: string;
  action?: ReactNode;
}

/** First-class empty state — every list screen must render one. */
export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className="status status--empty" role="status">
      <div className="status__icon" aria-hidden="true">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <path
            d="M3 5.5A2.5 2.5 0 0 1 5.5 3h9A2.5 2.5 0 0 1 17 5.5v9a2.5 2.5 0 0 1-2.5 2.5h-9A2.5 2.5 0 0 1 3 14.5v-9Z"
            stroke="currentColor"
            strokeWidth="1.5"
          />
          <path d="m6 6.5 2.5 2.5L6 11.5M10 12h4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </div>
      <h3 className="status__title">{title}</h3>
      {description && <p className="status__description">{description}</p>}
      {action && <div className="status__action">{action}</div>}
    </div>
  );
}

export interface ErrorStateProps {
  title?: string;
  message?: string;
  onRetry?: () => void;
  retryLabel?: string;
}

/** Error state with a retry action; the retry button stays focusable. */
export function ErrorState({
  title = "Something went wrong",
  message,
  onRetry,
  retryLabel = "Try again",
}: ErrorStateProps) {
  return (
    <div className="status status--error" role="alert">
      <div className="status__icon" aria-hidden="true">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="none">
          <path
            d="M10 3 1.8 17h16.4L10 3Z"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
          <path d="M10 8v4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          <circle cx="10" cy="14.5" r="0.5" fill="currentColor" />
        </svg>
      </div>
      <h3 className="status__title">{title}</h3>
      {message && <p className="status__description">{message}</p>}
      {onRetry && (
        <div className="status__action">
          <button type="button" className="status__retry" onClick={onRetry}>
            {retryLabel}
          </button>
        </div>
      )}
    </div>
  );
}

/** Inline spinner with an explicit accessible label (icon-only). */
export function Spinner({ label = "Loading" }: { label?: string }) {
  return (
    <span className="spinner" role="status" aria-label={label}>
      <span className="spinner__ring" aria-hidden="true" />
    </span>
  );
}
