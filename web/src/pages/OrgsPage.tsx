import { Link } from "react-router-dom";
import { useOrgs } from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import "./orgs.css";

function OrgsSkeleton() {
  return (
    <div className="orgs-list" data-testid="orgs-loading" aria-label="Loading organizations">
      {[0, 1, 2].map((i) => (
        <div className="org-card org-card--skeleton" key={i}>
          <Skeleton className="org-card__name" />
          <Skeleton className="org-card__desc" />
        </div>
      ))}
    </div>
  );
}

/** Organization directory — companies and teams on the platform. */
export function OrgsPage() {
  const { data, isLoading, isError, error, refetch } = useOrgs({ limit: 50 });
  const orgs = data?.organizations ?? [];

  return (
    <div className="orgs">
      <section className="orgs__hero">
        <h1 className="orgs__title">Organizations</h1>
        <p className="orgs__subtitle">Teams building in public — verified orgs, real people.</p>
      </section>

      {isLoading ? (
        <OrgsSkeleton />
      ) : isError ? (
        <ErrorState
          title="Couldn't load organizations"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : orgs.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No organizations yet"
          description="The directory fills in as teams create their org profiles."
        />
      ) : (
        <ul className="orgs-list">
          {orgs.map((o) => (
            <li key={o.id}>
              <Link to={`/orgs/${o.slug}`} className="org-card">
                <span className="org-card__name">{o.name}</span>
                {o.description && <span className="org-card__desc">{o.description}</span>}
                {o.industry && (
                  <span className="org-card__meta">{o.industry}</span>
                )}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
