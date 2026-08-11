import { Link, useLocation } from "react-router-dom";
import { cn } from "../lib/cn";
import "./breadcrumbs.css";

export interface Crumb {
  label: string;
  path?: string;
}

/**
 * Breadcrumbs — given the current crumb trail (built by the route map), the
 * last item is the current page (not a link), the rest link back.
 */
export function Breadcrumbs({ items, className }: { items: Crumb[]; className?: string }) {
  const location = useLocation();
  if (items.length === 0) return null;
  return (
    <nav aria-label="Breadcrumb" className={cn("breadcrumbs", className)}>
      <ol>
        {items.map((crumb, i) => {
          const isLast = i === items.length - 1;
          return (
            <li key={`${crumb.label}-${i}`}>
              {isLast || !crumb.path ? (
                <span
                  aria-current={isLast ? "page" : undefined}
                  className="breadcrumbs__current"
                >
                  {crumb.label}
                </span>
              ) : (
                <Link
                  to={crumb.path}
                  className="breadcrumbs__link"
                  aria-current={location.pathname === crumb.path ? "page" : undefined}
                >
                  {crumb.label}
                </Link>
              )}
              {!isLast && <span className="breadcrumbs__sep" aria-hidden="true">/</span>}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
