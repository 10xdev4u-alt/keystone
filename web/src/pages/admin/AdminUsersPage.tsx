import { Link } from "react-router-dom";
import { useAdminUsers } from "../../api/hooks";
import { ErrorState, Skeleton } from "../../components/Status/Status";
import "./admin.css";

function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const days = Math.floor(Math.max(0, Date.now() - then) / 86_400_000);
  if (days === 0) return "today";
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function AdminUsersPage() {
  const { data, isLoading, isError, error, refetch } = useAdminUsers({ limit: 50 });

  if (isLoading) {
    return (
      <div className="admin-page">
        <Skeleton className="admin-row" />
        <Skeleton className="admin-row" />
        <Skeleton className="admin-row" />
      </div>
    );
  }
  if (isError || !data) {
    return (
      <ErrorState
        title="Couldn't load the user directory"
        message={error instanceof Error ? error.message : "Admin access required."}
        onRetry={() => void refetch()}
      />
    );
  }

  return (
    <div className="admin-page">
      <header className="admin-page__header">
        <h1>Users</h1>
        <p className="admin-page__sub">{data.users.length} newest accounts.</p>
      </header>
      <div className="admin-table-wrap">
        <table className="admin-table">
          <thead>
            <tr>
              <th>User</th>
              <th>Role</th>
              <th>Status</th>
              <th>Verified</th>
              <th>Joined</th>
            </tr>
          </thead>
          <tbody>
            {data.users.map((u) => (
              <tr key={u.id}>
                <td>
                  <Link to={`/users/${u.id}`} className="admin-table__link">
                    {u.username ?? u.email.split("@")[0]}
                  </Link>
                  <span className="admin-table__email">{u.email}</span>
                </td>
                <td>
                  <span className="admin-badge" data-role={u.role}>
                    {u.role}
                  </span>
                </td>
                <td>{u.status}</td>
                <td>{u.is_verified ? "✓" : "—"}</td>
                <td>{timeAgo(u.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
