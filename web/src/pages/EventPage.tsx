import { Link, useParams } from "react-router-dom";
import { useEvent } from "../api/hooks";
import { ErrorState, Skeleton } from "../components/Status/Status";
import "./event.css";

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Event detail — the placeholder /events/:slug route, now real. */
export function EventPage() {
  const { slug = "" } = useParams<{ slug: string }>();
  const { data, isLoading, isError, error, refetch } = useEvent(slug);

  if (isLoading) {
    return (
      <div className="event">
        <Skeleton className="event__skeleton-title" />
        <Skeleton className="event__skeleton-line" />
        <Skeleton className="event__skeleton-line" />
      </div>
    );
  }

  if (isError || !data?.event) {
    return (
      <ErrorState
        title="Event unavailable"
        message={error instanceof Error ? error.message : "This event doesn't exist."}
        onRetry={() => void refetch()}
      />
    );
  }

  const { event, speakers, my_registration } = data;

  return (
    <article className="event">
      <header className="event__header">
        <p className="event__status" data-status={event.status}>
          {event.status}
        </p>
        <h1 className="event__title">{event.title}</h1>
        <p className="event__when">{formatDate(event.starts_at)}</p>
        {event.ends_at && <p className="event__when">until {formatDate(event.ends_at)}</p>}
        {event.location && <p className="event__where">📍 {event.location}</p>}
        {my_registration && (
          <p className="event__registered">
            You're {my_registration === "registered" ? "registered" : "on the waitlist"} for this
            event.
          </p>
        )}
      </header>

      {event.description && <div className="event__body">{event.description}</div>}

      <section className="event__meta">
        {event.capacity != null && (
          <p className="event__capacity">Capacity: {event.capacity}</p>
        )}
        {speakers.length > 0 && (
          <div className="event__speakers">
            <h2 className="event__subtitle">Speakers</h2>
            <ul>
              {speakers.map((s) => (
                <li key={s}>
                  <Link to={`/users/${s}`} className="event__speaker">
                    {s}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      <p className="event__back">
        <Link to="/events">← All events</Link>
      </p>
    </article>
  );
}
