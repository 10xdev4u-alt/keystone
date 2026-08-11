import { Navigate, Link } from "react-router-dom";
import { useCurrentUser } from "../api/hooks";
import { EmptyState, Skeleton } from "../components/Status/Status";

/**
 * /me/profile → the canonical public profile of the signed-in user.
 * The ProfilePage already renders bio, visibility and the owner edit form,
 * so this is a thin redirect rather than a duplicate screen.
 */
export function MyProfilePage() {
  const { data: me, isLoading } = useCurrentUser();

  if (isLoading) {
    return (
      <div className="profile">
        <Skeleton className="profile__skeleton-title" />
        <Skeleton className="profile__skeleton-line" />
      </div>
    );
  }

  if (!me?.id) {
    return (
      <div className="profile">
        <EmptyState
          headingLevel={2}
          title="Sign in to view your profile"
          description="Your public profile, connections and activity live here."
          action={<Link to="/login">Sign in</Link>}
        />
      </div>
    );
  }

  return <Navigate to={`/users/${me.id}`} replace />;
}
