import { useAdminStatus } from "../../api/hooks";
import { ErrorState, Skeleton } from "../../components/Status/Status";
import "./admin.css";

function formatUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function AdminOverviewPage() {
  const { data, isLoading, isError, error, refetch } = useAdminStatus();

  if (isLoading) {
    return (
      <div className="admin-page">
        <Skeleton className="admin-stat" />
        <Skeleton className="admin-stat" />
        <Skeleton className="admin-stat" />
      </div>
    );
  }
  if (isError || !data) {
    return (
      <ErrorState
        title="Couldn't load instance stats"
        message={error instanceof Error ? error.message : "Admin access required."}
        onRetry={() => void refetch()}
      />
    );
  }

  const stats = [
    { label: "Status", value: data.status, tone: data.status === "ok" ? "good" : "warn" },
    { label: "Registered users", value: data.users?.toLocaleString() ?? "—" },
    { label: "Live sessions", value: data.live_sessions?.toLocaleString() ?? "—" },
    { label: "Uptime", value: formatUptime(data.uptime_secs) },
  ];

  return (
    <div className="admin-page">
      <header className="admin-page__header">
        <h1>Instance overview</h1>
        <p className="admin-page__sub">Live platform health at a glance.</p>
      </header>
      <div className="admin-stats">
        {stats.map((s) => (
          <div key={s.label} className="admin-stat" data-tone={s.tone}>
            <span className="admin-stat__label">{s.label}</span>
            <span className="admin-stat__value">{s.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
