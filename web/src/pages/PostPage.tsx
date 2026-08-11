import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  useComments,
  useCreateComment,
  useCurrentUser,
  usePost,
  useRelatedPosts,
} from "../api/hooks";
import { EmptyState, ErrorState, Skeleton, Spinner } from "../components/Status/Status";
import "./post.css";

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

/** Sticky reading-progress bar — fills as the reader scrolls the article. */
function ReadingProgress() {
  const [progress, setProgress] = useState(0);
  useEffect(() => {
    function onScroll() {
      const doc = document.documentElement;
      const total = doc.scrollHeight - doc.clientHeight;
      setProgress(total > 0 ? Math.min(100, Math.round((window.scrollY / total) * 100)) : 0);
    }
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", onScroll);
    };
  }, []);
  return (
    <div className="reading-progress" role="progressbar" aria-label="Reading progress" aria-valuenow={progress} aria-valuemin={0} aria-valuemax={100}>
      <div className="reading-progress__bar" style={{ width: `${progress}%` }} />
    </div>
  );
}

/** Share actions — native share sheet where available, else copy link. */
function ShareActions({ title, url }: { title: string; url: string }) {
  const [copied, setCopied] = useState(false);
  const canShare = typeof navigator !== "undefined" && !!navigator.share;

  async function share() {
    if (canShare) {
      try {
        await navigator.share({ title, url });
        return;
      } catch {
        // user dismissed or share unsupported → fall through to copy
      }
    }
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      /* clipboard unavailable */
    }
  }

  return (
    <div className="share">
      <button type="button" className="share__btn" onClick={() => void share()}>
        {copied ? "✓ Copied" : canShare ? "Share" : "Copy link"}
      </button>
    </div>
  );
}

function PostSkeleton() {
  return (
    <div className="post" data-testid="post-loading" aria-label="Loading post">
      <Skeleton className="post__kind" />
      <Skeleton className="post__title" />
      <Skeleton className="post__body" />
      <Skeleton className="post__body" />
      <Skeleton className="post__body post__body--short" />
    </div>
  );
}

/** The reader view — full post body + comment thread. */
export function PostPage() {
  const { id = "" } = useParams();
  const {
    data: detail,
    isLoading: postLoading,
    isError: postError,
    error: postErrorObj,
    refetch: refetchPost,
  } = usePost(id);
  // Comments, related reading and the comment form all key off the canonical
  // post UUID (resolved from the detail response), never the URL slug — those
  // endpoints require a UUID.
  const canonicalId = detail?.post?.id ?? id;
  const {
    data: thread,
    isLoading: commentsLoading,
    isError: commentsError,
    refetch: refetchComments,
  } = useComments(canonicalId);

  const {
    data: related,
    isLoading: relatedLoading,
  } = useRelatedPosts(canonicalId);

  const { data: me, isLoading: meLoading } = useCurrentUser();
  const [comment, setComment] = useState("");
  const [commentError, setCommentError] = useState<string | null>(null);
  const createComment = useCreateComment(canonicalId);

  async function submitComment() {
    const body = comment.trim();
    if (!body) return;
    setCommentError(null);
    try {
      await createComment.mutateAsync({ body });
      setComment("");
    } catch (err) {
      setCommentError(err instanceof Error ? err.message : "Could not post comment");
    }
  }

  const post = detail?.post;

  return (
    <div className="post-page">
      {postLoading ? (
        <PostSkeleton />
      ) : postError || !post ? (
        <ErrorState
          title="Couldn't load this post"
          message={postErrorObj instanceof Error ? postErrorObj.message : "Not found or not visible"}
          onRetry={() => void refetchPost()}
        />
      ) : (
        <>
          <ReadingProgress />
          <article className="post">
            <div className="post__top">
              <span className="post__kind" data-kind={post.kind}>
                {post.kind}
              </span>
              <span className="post__visibility">{post.visibility}</span>
            </div>
            <h1 className="post__title">{post.title}</h1>
            <div className="post__meta">
              {post.published_at && <time dateTime={post.published_at}>{timeAgo(post.published_at)}</time>}
              <span>{post.view_count} views</span>
              <span>by {post.author_id.slice(0, 8)}</span>
              <ShareActions title={post.title} url={window.location.href} />
            </div>
            <div className="post__body">{post.body}</div>
          </article>

          <section className="comments" aria-labelledby="comments-heading">
            <h2 id="comments-heading" className="comments__heading">
              Comments
            </h2>
            {commentsLoading ? (
              <div className="comments__loading" aria-label="Loading comments">
                <Spinner label="Loading comments" />
              </div>
            ) : commentsError ? (
              <ErrorState
                title="Couldn't load comments"
                onRetry={() => void refetchComments()}
              />
            ) : (thread?.comments.length ?? 0) === 0 ? (
              <EmptyState headingLevel={3} title="No comments yet" />
            ) : (
              <ul className="comments__list">
                {thread?.comments.map((c) => (
                  <li key={c.id} className="comment">
                    <div className="comment__head">
                      <Link to={`/users/${c.author_id}`} className="comment__author">
                        {c.author_id.slice(0, 8)}
                      </Link>
                      <time className="comment__time" dateTime={c.created_at}>
                        {timeAgo(c.created_at)}
                      </time>
                    </div>
                    <p className="comment__body">{c.body}</p>
                  </li>
                ))}
              </ul>
            )}

            {meLoading ? null : me ? (
              <form
                className="comment-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void submitComment();
                }}
              >
                <label className="comment-form__label" htmlFor="comment-body">
                  Add a comment
                </label>
                <textarea
                  id="comment-body"
                  className="comment-form__input"
                  rows={3}
                  value={comment}
                  onChange={(e) => setComment(e.target.value)}
                  placeholder="Share your thoughts…"
                />
                {commentError && <p className="comment-form__error" role="alert">{commentError}</p>}
                <div className="comment-form__actions">
                  <button
                    type="submit"
                    className="comment-form__submit"
                    disabled={createComment.isPending || comment.trim().length === 0}
                  >
                    {createComment.isPending ? "Posting…" : "Post comment"}
                  </button>
                </div>
              </form>
            ) : (
              <div className="comment-form__hint">
                <Link to="/login">Sign in</Link> to join the discussion.
              </div>
            )}
          </section>

          <RelatedSection loading={relatedLoading} posts={related?.posts ?? []} />
        </>
      )}
    </div>
  );
}

function RelatedSection({
  loading,
  posts,
}: {
  loading: boolean;
  posts: { id: string; kind: string; title: string; slug: string; summary?: string | null }[];
}) {
  if (posts.length === 0) return null;
  return (
    <section className="related" aria-labelledby="related-heading">
      <h2 id="related-heading" className="related__heading">
        Related reading
      </h2>
      {loading ? (
        <Skeleton className="related__skeleton" />
      ) : (
        <ul className="related__list">
          {posts.map((p) => (
            <li key={p.id} className="related__item">
              <Link to={`/posts/${p.slug}`} className="related__link">
                <span className="related__kind" data-kind={p.kind}>
                  {p.kind}
                </span>
                <h3 className="related__title">{p.title}</h3>
                {p.summary && <p className="related__summary">{p.summary}</p>}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
