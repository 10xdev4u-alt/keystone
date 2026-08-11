import { Link } from "react-router-dom";
import { Button } from "../components/Button/Button";
import { EmptyState } from "../components/Status/Status";

export function NotFound() {
  return (
    <EmptyState
      title="Page not found"
      description="The page you're looking for doesn't exist or has moved."
      action={
        <Button asChild>
          <Link to="/">Back to home</Link>
        </Button>
      }
    />
  );
}
