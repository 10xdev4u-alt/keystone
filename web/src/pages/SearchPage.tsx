import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useSearch } from "../api/hooks";
import { EmptyState, ErrorState, Spinner } from "../components/Status/Status";
import "./search.css";

const DEBOUNCE_MS = 300;

function resultHref(hit: { entity_type: string; entity_id: string }): string {
  switch (hit.entity_type) {
    case "post":
      return `/posts/${hit.entity_id}`;
    case "community":
      return `/communities/${hit.entity_id}`;
    case "course":
      return `/courses/${hit.entity_id}`;
    default:
      return `/users/${hit.entity_id}`;
  }
}

/** Unified search — debounced as-you-type with typed results. */
export function SearchPage() {
  const [params, setParams] = useSearchParams();
  const urlQuery = params.get("q") ?? "";
  const [input, setInput] = useState(urlQuery);
  const [debounced, setDebounced] = useState(urlQuery);

  // Sync the URL query to the debounced value (back/forward friendly).
  useEffect(() => {
    const t = setTimeout(() => {
      setDebounced(input);
      setParams(input.trim() ? { q: input.trim() } : {}, { replace: true });
    }, DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [input, setParams]);

  const { data, isLoading, isError, error, refetch, isFetching } = useSearch(debounced.trim());

  return (
    <div className="search">
      <section className="search__hero">
        <h1 className="search__title">Search</h1>
        <p className="search__subtitle">Posts, communities, courses and people — one box.</p>
        <label className="search__label" htmlFor="search-input">
          Search the platform
        </label>
        <input
          id="search-input"
          className="search__input"
          type="search"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Try “rust” or “ownership”…"
          autoFocus
        />
      </section>

      {debounced.trim().length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="Type to search"
          description="Results appear as you type — typo-tolerant full-text search across everything."
        />
      ) : isLoading ? (
        <div className="search__loading" aria-label="Searching">
          <Spinner label="Searching" />
        </div>
      ) : isError ? (
        <ErrorState
          title="Search failed"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : (data?.results.length ?? 0) === 0 ? (
        <EmptyState
          headingLevel={2}
          title={`No results for “${debounced.trim()}”`}
          description="Try different keywords or check the spelling."
        />
      ) : (
        <>
          <p className="search__count" aria-live="polite">
            {data?.results.length} result{data?.results.length === 1 ? "" : "s"}
            {isFetching ? "…" : ""} for “{debounced.trim()}”
          </p>
          <ul className="search__results">
            {data?.results.map((hit, i) => (
              <li key={`${hit.entity_type}-${hit.entity_id}-${i}`}>
                <Link to={resultHref(hit)} className="search-hit">
                  <span className="search-hit__type" data-type={hit.entity_type}>
                    {hit.entity_type}
                  </span>
                  <span className="search-hit__title">{hit.title}</span>
                  {hit.snippet && (
                    <span className="search-hit__snippet">
                      {/* ts_headline returns <b> markup — render as plain text,
                          never as HTML (defense in depth against XSS). */}
                      {hit.snippet.replace(/<[^>]+>/g, "")}
                    </span>
                  )}
                </Link>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
