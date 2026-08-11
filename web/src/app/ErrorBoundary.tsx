import { isRouteErrorResponse, useRouteError } from "react-router-dom";
import { ErrorState } from "../components/Status/Status";

/**
 * Route-level error boundary (React Router errorElement). A thrown error or
 * a 404/500 from a loader renders the design system's ErrorState instead of
 * a blank screen. The page URL is preserved so refresh re-attempts.
 */
export function RouteErrorBoundary() {
  const error = useRouteError();
  let title = "Something went wrong";
  let message = "An unexpected error occurred. Try again.";

  if (isRouteErrorResponse(error)) {
    title = `${error.status} ${error.statusText}`;
    message = "The page you're looking for doesn't exist or moved.";
  } else if (error instanceof Error) {
    message = error.message;
  }

  return (
    <ErrorState
      title={title}
      message={message}
      onRetry={() => window.location.reload()}
      retryLabel="Reload page"
    />
  );
}
