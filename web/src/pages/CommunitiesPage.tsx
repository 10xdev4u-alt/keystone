import { Link } from "react-router-dom";
import { useCommunities } from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import "./communities.css";

function CommunitySkeleton() {
  return (
    <div className="community-list" data-testid="communities-loading" aria-label="Loading communities">
      {[0, 1, 2].map((i) => (
        <div className="community-card community-card--skeleton" key={i}>
          <Skeleton className="community-card__name" />
          <Skeleton className="community-card__desc" />
        </div>
      ))}
    </div>
  );
}

/** Community directory — every public space, paged. */
export function CommunitiesPage() {
  const { data, isLoading, isError, error, refetch } = useCommunities({ limit: 50 });
  const communities = data?.communities ?? [];

  return (
    <div className="communities">
      <section className="communities__hero">
        <h1 className="communities__title">Communities</h1>
        <p className="communities__subtitle">
          Topic spaces for focused discussion — join one and jump in.
        </p>
      </section>

      {isLoading ? (
        <CommunitySkeleton />
      ) : isError ? (
        <ErrorState
          title="Couldn't load communities"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : communities.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No communities yet"
          description="The directory fills in as communities are created."
        />
      ) : (
        <ul className="community-list">
          {communities.map((c) => (
            <li key={c.id}>
              <Link to={`/communities/${c.slug}`} className="community-card">
                <span className="community-card__name">{c.name}</span>
                {c.description && (
                  <span className="community-card__desc">{c.description}</span>
                )}
                <span className="community-card__meta">
                  {c.visibility} · created {new Date(c.created_at).toLocaleDateString()}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
