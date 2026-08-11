import { useState } from "react";
import {
  useCurrentUser,
  useRevokeAllSessions,
  useRevokeSession,
  useSessions,
} from "../api/hooks";
import { EmptyState, ErrorState, Spinner } from "../components/Status/Status";
import "./sessions.css";

/** Human label for a session's user-agent — best effort, never blocks. */
function deviceLabel(ua: string | null | undefined): string {
  if (!ua) return "Unknown device";
  if (/iPhone|Android|iPad/i.test(ua)) return "Mobile device";
  if (/Macintosh|Mac OS X/i.test(ua)) return "Mac";
  if (/Windows/i.test(ua)) return "Windows PC";
  if (/Linux/i.test(ua)) return "Linux computer";
  return "Web browser";
}

function timeAgo(iso: string): string {
  const seconds = Math.max(1, Math.floor((Date.now() - Date.parse(iso)) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

/** Security surface: every live session, revoke one or all. */
export function SessionsPage() {
  const { data: me } = useCurrentUser();
  const { data, isLoading, isError, error, refetch } = useSessions();
  const revokeSession = useRevokeSession();
  const revokeAll = useRevokeAllSessions();
  const [confirmingAll, setConfirmingAll] = useState(false);
  const [revokingId, setRevokingId] = useState<string | null>(null);

  if (!me) {
    return (
      <EmptyState
        title="Sign in required"
        description="Sign in to see and manage your active sessions."
      />
    );
  }

  return (
    <div className="sessions">
      <header className="sessions__header">
        <h1 className="sessions__title">Active sessions</h1>
        <p className="sessions__subtitle">
          Every device signed into your account. Revoke anything you don't
          recognize — the device is signed out immediately.
        </p>
      </header>

      {isLoading ? (
        <Spinner label="Loading sessions" />
      ) : isError ? (
        <ErrorState
          title="Couldn't load sessions"
          message={error instanceof Error ? error.message : "Unknown error"}
          onRetry={() => void refetch()}
        />
      ) : !data || data.sessions.length === 0 ? (
        <EmptyState
          title="No active sessions"
          description="Sessions appear here the first time you sign in."
        />
      ) : (
        <>
          <ul className="sessions__list" aria-label="Active sessions">
            {data.sessions.map((session) => (
              <li className="sessions__item" key={session.id}>
                <div className="sessions__item-main">
                  <div className="sessions__item-head">
                    <strong className="sessions__device">
                      {deviceLabel(session.user_agent)}
                    </strong>
                    {session.current && (
                      <span className="sessions__badge">This device</span>
                    )}
                  </div>
                  <div className="sessions__meta">
                    <span>Signed in {timeAgo(session.created_at)}</span>
                    {session.ip_address && <span>· {session.ip_address}</span>}
                    {session.user_agent && (
                      <span className="sessions__ua" title={session.user_agent}>
                        · {session.user_agent.slice(0, 60)}
                      </span>
                    )}
                  </div>
                  <div className="sessions__meta">
                    <span>Expires {timeAgo(session.expires_at)}</span>
                  </div>
                </div>
                <button
                  type="button"
                  className="btn btn--ghost btn--sm sessions__revoke"
                  disabled={revokingId === session.id || session.current}
                  onClick={() => {
                    setRevokingId(session.id);
                    revokeSession.mutate(session.id, {
                      onSettled: () => setRevokingId(null),
                    });
                  }}
                >
                  {revokingId === session.id ? "Revoking…" : "Revoke"}
                </button>
              </li>
            ))}
          </ul>

          <div className="sessions__danger">
            {confirmingAll ? (
              <>
                <p className="sessions__danger-text">
                  This signs out every device, including this one. You'll need
                  to sign in again.
                </p>
                <div className="sessions__danger-actions">
                  <button
                    type="button"
                    className="btn btn--danger btn--sm"
                    onClick={() => revokeAll.mutate()}
                  >
                    {revokeAll.isPending ? "Signing out…" : "Yes, sign out everywhere"}
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost btn--sm"
                    onClick={() => setConfirmingAll(false)}
                  >
                    Cancel
                  </button>
                </div>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => setConfirmingAll(true)}
              >
                Sign out of all devices
              </button>
            )}
          </div>
        </>
      )}
    </div>
  );
}
