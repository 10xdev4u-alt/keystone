import { Link, useParams } from "react-router-dom";
import { useOrg } from "../api/hooks";
import { ErrorState, Skeleton } from "../components/Status/Status";
import "./orgs.css";

/** Organization profile — verified team page. */
export function OrgPage() {
  const { slug = "" } = useParams();
  const { data, isLoading, isError, error, refetch } = useOrg(slug);
  const org = data?.organization;

  if (isLoading) {
    return (
      <div className="org" data-testid="org-loading" aria-label="Loading organization">
        <Skeleton className="org__title" />
        <Skeleton className="org__desc" />
        <Skeleton className="org__desc org__desc--short" />
      </div>
    );
  }

  if (isError || !org) {
    return (
      <ErrorState
        title="Couldn't load this organization"
        message={error instanceof Error ? error.message : "Not found"}
        onRetry={() => void refetch()}
      />
    );
  }

  return (
    <div className="org">
      <header className="org__header">
        <h1 className="org__title">{org.name}</h1>
        {org.industry && <span className="org-card__meta">{org.industry}</span>}
        {org.description && <p className="org__desc">{org.description}</p>}
      </header>

      <dl className="org__facts">
        {org.website && (
          <div className="org__fact">
            <dt>Website</dt>
            <dd>
              <a href={org.website} target="_blank" rel="noopener noreferrer">
                {org.website}
              </a>
            </dd>
          </div>
        )}
        <div className="org__fact">
          <dt>Joined</dt>
          <dd>{new Date(org.created_at).toLocaleDateString()}</dd>
        </div>
        <div className="org__fact">
          <dt>Slug</dt>
          <dd>{org.slug}</dd>
        </div>
      </dl>

      <Link to="/orgs" className="org__back">
        ← All organizations
      </Link>
    </div>
  );
}
