import { useState } from "react";
import { Link } from "react-router-dom";
import { keepPreviousData } from "@tanstack/react-query";
import { usePosts } from "../api/hooks";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import { cn } from "../lib/cn";
import "./home.css";

/** Relative time — "3h ago" style; stable and locale-friendly. */
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

function PostCard({
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
    <article className="post-card" data-kind={kind}>
      <div className="post-card__top">
        <span className="post-card__kind" data-kind={kind}>
          {kind}
        </span>
        {publishedAt && <time className="post-card__time">{timeAgo(publishedAt)}</time>}
      </div>
      <h2 className="post-card__title">
        <Link to={`/posts/${id}`} className="post-card__link">
          {title}
        </Link>
      </h2>
      {summary && <p className="post-card__summary">{summary}</p>}
      <div className="post-card__meta" aria-label={`${commentCount} comments, ${reactionCount} reactions, ${viewCount} views`}>
        <span className="post-card__stat">
          <span aria-hidden="true">💬</span> {commentCount}
        </span>
        <span className="post-card__stat">
          <span aria-hidden="true">❤️</span> {reactionCount}
        </span>
        <span className="post-card__stat">
          <span aria-hidden="true">👁</span> {viewCount}
        </span>
      </div>
    </article>
  );
}

function FeedSkeleton() {
  return (
    <div className="feed" data-testid="feed-loading" aria-label="Loading posts">
      {[0, 1, 2, 3].map((i) => (
        <div className="post-card post-card--skeleton" key={i}>
          <Skeleton className="post-card__kind" />
          <Skeleton className="post-card__title" />
          <Skeleton className="post-card__summary" />
          <Skeleton className="post-card__meta" />
        </div>
      ))}
    </div>
  );
}

/** The homepage — newest posts across every kind, keyset-paginated. */
export function HomePage() {
  const [before, setBefore] = useState<string | undefined>(undefined);
  const { data, isLoading, isError, error, isFetching, refetch } = usePosts(
    { limit: 20, before },
    { placeholderData: keepPreviousData },
  );
  const posts = data?.posts ?? [];

  return (
    <div className="home">
      <section className="home__hero">
        <h1 className="home__title">Fresh from the community</h1>
        <p className="home__subtitle">
          Articles, questions, polls and discussions — all built on a fully-typed API.
        </p>
      </section>

      {isLoading ? (
        <FeedSkeleton />
      ) : isError ? (
        <ErrorState
          title="Couldn't load the feed"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : posts.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No posts yet"
          description="Be the first to publish — the feed fills in as content lands."
        />
      ) : (
        <>
          <div className={cn("feed", isFetching && "feed--refreshing")} aria-busy={isFetching || undefined}>
            {posts.map((post) => (
              <PostCard
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
            <button type="button" className="home__more" onClick={() => setBefore(data.next_cursor!)}>
              Load older posts
            </button>
          )}
        </>
      )}
    </div>
  );
}
