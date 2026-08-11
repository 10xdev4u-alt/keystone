import { Link, useParams } from "react-router-dom";
import {
  useCommunity,
  useCommunityMembers,
  useCommunityPosts,
  useCurrentUser,
  useJoinCommunity,
} from "../api/hooks";
import { EmptyState, ErrorState, Skeleton, Spinner } from "../components/Status/Status";
import "./community.css";

/** Community home — detail, membership, and its posts. */
export function CommunityPage() {
  const { slug = "" } = useParams();
  const {
    data: detail,
    isLoading: loading,
    isError,
    error,
    refetch,
  } = useCommunity(slug);
  const { data: members, isLoading: membersLoading } = useCommunityMembers(slug);
  const { data: posts } = useCommunityPosts(slug);
  const { data: me, isLoading: meLoading } = useCurrentUser();
  const join = useJoinCommunity(slug);

  const community = detail?.community;
  const isMember = me
    ? members?.members.some((m) => m.user_id === me.id)
    : false;

  if (loading || meLoading) {
    return (
      <div className="community" data-testid="community-loading" aria-label="Loading community">
        <Skeleton className="community__title" />
        <Skeleton className="community__desc" />
        <Skeleton className="community__desc community__desc--short" />
      </div>
    );
  }

  if (isError || !community) {
    return (
      <ErrorState
        title="Couldn't load this community"
        message={error instanceof Error ? error.message : "Not found"}
        onRetry={() => void refetch()}
      />
    );
  }

  return (
    <div className="community">
      <header className="community__header">
        <div className="community__headline">
          <span className="community__visibility">{community.visibility}</span>
          <h1 className="community__title">{community.name}</h1>
          {community.description && (
            <p className="community__desc">{community.description}</p>
          )}
        </div>
        <div className="community__actions">
          {me && !isMember && (
            <button
              type="button"
              className="community__join"
              disabled={join.isPending}
              onClick={() => void join.mutateAsync()}
            >
              {join.isPending ? "Joining…" : "Join community"}
            </button>
          )}
          {me && isMember && (
            <span className="community__joined" role="status">
              ✓ Member
            </span>
          )}
        </div>
      </header>

      <section className="community__section" aria-labelledby="members-heading">
        <h2 id="members-heading" className="community__section-title">
          Members
        </h2>
        {membersLoading ? (
          <div className="community__loading" aria-label="Loading members">
            <Spinner label="Loading members" />
          </div>
        ) : (members?.members.length ?? 0) === 0 ? (
          <EmptyState headingLevel={3} title="No members yet" />
        ) : (
          <ul className="community__members">
            {members?.members.map((m) => (
              <li key={m.user_id} className="community__member">
                <span className="community__member-id">{m.user_id.slice(0, 8)}</span>
                <span className="community__member-role">{m.role}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="community__section" aria-labelledby="posts-heading">
        <h2 id="posts-heading" className="community__section-title">
          Posts
        </h2>
        {(posts?.posts.length ?? 0) === 0 ? (
          <EmptyState headingLevel={3} title="No posts here yet" />
        ) : (
          <ul className="community__posts">
            {posts?.posts.map((p) => (
              <li key={p.post_id}>
                <Link to={`/posts/${p.post_id}`} className="community__post">
                  <span>{p.pinned ? "📌 " : ""}Post</span>
                  <span className="community__post-meta">{p.post_id.slice(0, 8)}</span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
