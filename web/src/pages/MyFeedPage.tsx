import { useState } from "react";
import { Link } from "react-router-dom";
import { keepPreviousData } from "@tanstack/react-query";
import { useCurrentUser, usePosts } from "../api/hooks";
import { PostComposer } from "../components/PostComposer/PostComposer";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import { cn } from "../lib/cn";
import "./myfeed.css";

/** Relative time — "3h ago" style; mirrors the home feed. */
function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function MyPostCard({
  id,
  kind,
  title,
  summary,
  viewCount,
  commentCount,
  reactionCount,
  publishedAt,
}: {
  id: string;
  kind: string;
  title: string;
  summary?: string | null;
  viewCount: number;
  commentCount: number;
  reactionCount: number;
  publishedAt?: string | null;
}) {
  return (
    <article className="myfeed-card" data-kind={kind}>
      <div className="myfeed-card__top">
        <span className="myfeed-card__kind" data-kind={kind}>
          {kind}
        </span>
        {publishedAt && <time className="myfeed-card__time">{timeAgo(publishedAt)}</time>}
      </div>
      <h3 className="myfeed-card__title">
        <Link to={`/posts/${id}`} className="myfeed-card__link">
          {title}
        </Link>
      </h3>
      {summary && <p className="myfeed-card__summary">{summary}</p>}
      <div
        className="myfeed-card__meta"
        aria-label={`${commentCount} comments, ${reactionCount} reactions, ${viewCount} views`}
      >
        <span>💬 {commentCount}</span>
        <span>❤️ {reactionCount}</span>
        <span>👁 {viewCount}</span>
      </div>
    </article>
  );
}

/** The authenticated user's own feed — their posts + the composer. */
export function MyFeedPage() {
  const { data: me, isLoading: meLoading } = useCurrentUser();
  const [before, setBefore] = useState<string | undefined>(undefined);
  const { data, isLoading, isError, error, isFetching, refetch } = usePosts(
    { author: me?.id, limit: 20, before },
    { enabled: Boolean(me?.id), placeholderData: keepPreviousData },
  );
  const posts = data?.posts ?? [];

  if (meLoading) {
    return (
      <div className="myfeed">
        <Skeleton className="myfeed__composer-skeleton" />
      </div>
    );
  }

  if (!me) {
    return (
      <div className="myfeed">
        <EmptyState
          headingLevel={2}
          title="Sign in to see your feed"
          description="Your posts, drafts and published writing live here once you're signed in."
          action={<Link to="/login">Sign in</Link>}
        />
      </div>
    );
  }

  return (
    <div className="myfeed">
      <header className="myfeed__header">
        <h1 className="myfeed__title">My feed</h1>
        <p className="myfeed__subtitle">Write and manage your own posts.</p>
      </header>

      <PostComposer />

      <section className="myfeed__list" aria-label="Your posts">
        <h2 className="myfeed__section">Your posts</h2>
        {isLoading ? (
          <div className="myfeed__loading" data-testid="myfeed-loading" aria-label="Loading your posts">
            {[0, 1].map((i) => (
              <div className="myfeed-card myfeed-card--skeleton" key={i}>
                <Skeleton className="myfeed-card__kind" />
                <Skeleton className="myfeed-card__title" />
                <Skeleton className="myfeed-card__meta" />
              </div>
            ))}
          </div>
        ) : isError ? (
          <ErrorState
            title="Couldn't load your posts"
            message={error instanceof Error ? error.message : undefined}
            onRetry={() => void refetch()}
          />
        ) : posts.length === 0 ? (
          <EmptyState
            headingLevel={3}
            title="Nothing published yet"
            description="Write your first post above — it will show up here and on the home feed."
          />
        ) : (
          <>
            <div
              className={cn("myfeed__cards", isFetching && "myfeed__cards--refreshing")}
              aria-busy={isFetching || undefined}
            >
              {posts.map((post) => (
                <MyPostCard
                  key={post.id}
                  id={post.id}
                  kind={post.kind}
                  title={post.title}
                  summary={post.summary}
                  viewCount={post.view_count}
                  commentCount={post.comment_count}
                  reactionCount={post.reaction_count}
                  publishedAt={post.published_at}
                />
              ))}
            </div>
            {data?.next_cursor && (
              <button type="button" className="myfeed__more" onClick={() => setBefore(data.next_cursor!)}>
                Load older posts
              </button>
            )}
          </>
        )}
      </section>
    </div>
  );
}
