import { useOnline } from "../../hooks/useOnline";
import "./offline-indicator.css";

/**
 * Sticky banner shown while offline. Rendered in every shell so the user
 * always knows reads are stale / writes are queued locally.
 */
export function OfflineIndicator() {
  const online = useOnline();
  if (online) return null;
  return (
    <div className="offline" role="status" aria-live="polite">
      <span className="offline__dot" aria-hidden="true" />
      You're offline — showing saved content. Reconnect to see updates.
    </div>
  );
}
