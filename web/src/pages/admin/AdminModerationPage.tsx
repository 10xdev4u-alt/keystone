import { useState } from "react";
import { useReportQueue, useResolveReport } from "../../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../../components/Status/Status";
import "./admin.css";

function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 3600) return `${Math.max(1, Math.floor(secs / 60))}m ago`;
  const hours = Math.floor(secs / 3600);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function AdminModerationPage() {
  const { data, isLoading, isError, error, refetch } = useReportQueue({ limit: 50 });
  const resolve = useResolveReport({ onSuccess: () => void refetch() });
  const [note, setNote] = useState<Record<string, string>>({});
  const [resolvingId, setResolvingId] = useState<string | null>(null);

  if (isLoading) {
    return (
      <div className="admin-page">
        <Skeleton className="admin-row" />
        <Skeleton className="admin-row" />
        <Skeleton className="admin-row" />
      </div>
    );
  }
  if (isError || !data) {
    return (
      <ErrorState
        title="Couldn't load the queue"
        message={error instanceof Error ? error.message : "Moderator access required."}
        onRetry={() => void refetch()}
      />
    );
  }

  async function resolveReport(id: string) {
    setResolvingId(id);
    try {
      await resolve.mutateAsync({ id, resolution_note: note[id]?.trim() || undefined });
    } finally {
      setResolvingId(null);
    }
  }

  return (
    <div className="admin-page">
      <header className="admin-page__header">
        <h1>Moderation queue</h1>
        <p className="admin-page__sub">Open reports, newest first. Every resolution is recorded.</p>
      </header>

      {data.reports.length === 0 ? (
        <EmptyState headingLevel={2} title="Queue is clear" />
      ) : (
        <ul className="admin-list">
          {data.reports.map((r) => (
            <li key={r.id} className="admin-card">
              <div className="admin-card__head">
                <span className="admin-badge" data-type={r.entity_type}>
                  {r.entity_type}
                </span>
                <span className="admin-card__meta">
                  reported {timeAgo(r.created_at)} · #{r.entity_id.slice(0, 8)}
                </span>
              </div>
              <p className="admin-card__reason">{r.reason}</p>
              {r.detail && <p className="admin-card__detail">{r.detail}</p>}
              <div className="admin-card__actions">
                <input
                  className="admin-card__note"
                  placeholder="Resolution note (optional)"
                  value={note[r.id] ?? ""}
                  onChange={(e) => setNote((prev) => ({ ...prev, [r.id]: e.target.value }))}
                />
                <button
                  type="button"
                  className="btn btn--primary"
                  disabled={resolvingId === r.id || resolve.isPending}
                  onClick={() => void resolveReport(r.id)}
                >
                  {resolvingId === r.id ? "Resolving…" : "Resolve"}
                </button>
              </div>
              {resolve.error && <p className="admin-card__error" role="alert">{resolve.error.message}</p>}
            </li>
          ))}
        </ul>
      )}

    </div>
  );
}
