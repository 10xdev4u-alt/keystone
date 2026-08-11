import { Link } from "react-router-dom";
import { useEvents } from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import "./events.css";

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function EventsSkeleton() {
  return (
    <div className="events-list" data-testid="events-loading" aria-label="Loading events">
      {[0, 1, 2].map((i) => (
        <div className="event-card event-card--skeleton" key={i}>
          <Skeleton className="event-card__date" />
          <Skeleton className="event-card__title" />
          <Skeleton className="event-card__meta" />
        </div>
      ))}
    </div>
  );
}

/** Upcoming events — meetups, workshops, and talks. */
export function EventsPage() {
  const { data, isLoading, isError, error, refetch } = useEvents({ limit: 50 });
  const events = data?.events ?? [];

  return (
    <div className="events">
      <section className="events__hero">
        <h1 className="events__title">Events</h1>
        <p className="events__subtitle">Meetups, workshops and talks from the community.</p>
      </section>

      {isLoading ? (
        <EventsSkeleton />
      ) : isError ? (
        <ErrorState
          title="Couldn't load events"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : events.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No events yet"
          description="Upcoming events will appear here as organizers publish them."
        />
      ) : (
        <ul className="events-list">
          {events.map((e) => (
            <li key={e.id}>
              <Link to={`/events/${e.slug}`} className="event-card">
                <div className="event-card__date">
                  <span className="event-card__month">
                    {formatDate(e.starts_at).split(" ")[0]}
                  </span>
                  <span className="event-card__day">
                    {formatDate(e.starts_at).split(" ")[1]?.replace(",", "")}
                  </span>
                </div>
                <div className="event-card__body">
                  <span className="event-card__title">{e.title}</span>
                  <span className="event-card__meta">
                    {formatTime(e.starts_at)}
                    {e.location ? ` · ${e.location}` : ""}
                    {e.capacity != null ? ` · ${e.capacity} spots` : ""}
                  </span>
                </div>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
