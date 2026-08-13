import { useState } from "react";
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

const ROLES = ["user", "moderator", "admin", "super_admin"] as const;

export function AdminUsersPage() {
  const { data, isLoading, isError, error, refetch } = useAdminUsers({ limit: 100 });
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<string>("all");

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

  const needle = query.trim().toLowerCase();
  const users = data.users.filter((u) => {
    if (role !== "all" && u.role !== role) return false;
    if (!needle) return true;
    return (
      (u.username ?? "").toLowerCase().includes(needle) ||
      u.email.toLowerCase().includes(needle) ||
      u.role.includes(needle)
    );
  });

  return (
    <div className="admin-page">
      <header className="admin-page__header">
        <h1>Users</h1>
        <p className="admin-page__sub">
          {users.length} of {data.users.length} newest accounts.
        </p>
      </header>
      <div className="admin-filters">
        <input
          type="search"
          aria-label="Search users"
          placeholder="Search by username or email…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select aria-label="Filter by role" value={role} onChange={(e) => setRole(e.target.value)}>
          <option value="all">All roles</option>
          {ROLES.map((r) => (
            <option key={r} value={r}>
              {r}
            </option>
          ))}
        </select>
      </div>
      {users.length === 0 ? (
        <p className="admin-empty">No users match this filter.</p>
      ) : (
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
              {users.map((u) => (
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
      )}
    </div>
  );
}
