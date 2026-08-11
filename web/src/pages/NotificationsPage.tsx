import { Link } from "react-router-dom";
import {
  useMarkNotificationsRead,
  useNotifications,
} from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import { cn } from "../lib/cn";
import "./notifications.css";

/** Relative time — mirrors the feed pages. */
function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** A human-readable line for a notification's payload (best-effort). */
function describePayload(payload: unknown, kind: string): string {
  if (!payload || typeof payload !== "object") return kind;
  const record = payload as Record<string, unknown>;
  const candidate =
    typeof record.text === "string"
      ? record.text
      : typeof record.title === "string"
        ? record.title
        : typeof record.message === "string"
          ? record.message
          : null;
  return candidate ?? kind;
}

/** Link targets based on the notification's entity type. */
function entityHref(entityType: string, entityId: string | null): string | null {
  if (!entityId) return null;
  switch (entityType) {
    case "post":
    case "question":
      return `/posts/${entityId}`;
    case "comment":
      return `/posts/${entityId}`;
    case "community":
      return `/communities/${entityId}`;
    case "org":
      return `/orgs/${entityId}`;
    case "conversation":
      return `/me/conversations`;
    default:
      return null;
  }
}

/** The notifications center — unread first, mark-as-read, all-clear state. */
export function NotificationsPage() {
  const { data, isLoading, isError, error, refetch } = useNotifications();
  const markRead = useMarkNotificationsRead({
    onSuccess: () => {
      // Nothing extra — the hook invalidates the feed on success.
    },
  });

  const notifications = data?.notifications ?? [];
  const unread = data?.unread ?? 0;

  return (
    <div className="notifications">
      <header className="notifications__header">
        <div>
          <h1 className="notifications__title">Notifications</h1>
          {unread > 0 && (
            <p className="notifications__subtitle" aria-live="polite">
              {unread} unread
            </p>
          )}
        </div>
        {unread > 0 && (
          <button
            type="button"
            className="notifications__mark-all"
            onClick={() => markRead.mutate({ up_to: null })}
            disabled={markRead.isPending}
          >
            {markRead.isPending ? "Marking…" : "Mark all read"}
          </button>
        )}
      </header>

      {isLoading ? (
        <div className="notifications__list" data-testid="notifications-loading" aria-label="Loading notifications">
          {[0, 1, 2].map((i) => (
            <div className="notification notification--skeleton" key={i}>
              <Skeleton className="notification__line" />
            </div>
          ))}
        </div>
      ) : isError ? (
        <ErrorState
          title="Couldn't load notifications"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : notifications.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="You're all caught up"
          description="Replies, reactions and mentions will land here."
        />
      ) : (
        <ul className="notifications__list">
          {notifications.map((n) => {
            const href = entityHref(n.entity_type, n.entity_id ?? null);
            const inner = (
              <>
                <span className="notification__dot" aria-hidden="true" data-read={n.is_read || undefined} />
                <span className="notification__body">
                  <span className="notification__text" aria-current={n.is_read ? undefined : "true"}>
                    {describePayload(n.payload, n.kind)}
                  </span>
                  <span className="notification__meta">
                    <span className="notification__kind">{n.kind}</span>
                    {n.created_at && <time>{timeAgo(n.created_at)}</time>}
                  </span>
                </span>
              </>
            );
            const cls = cn("notification", !n.is_read && "notification--unread");
            return (
              <li key={n.id} className={cls}>
                {href ? (
                  <Link to={href} className="notification__link" onClick={() => markRead.mutate({ up_to: n.id })}>
                    {inner}
                  </Link>
                ) : (
                  <div className="notification__plain">{inner}</div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
