import { Link, Outlet } from "react-router-dom";
import { OfflineIndicator } from "../components/OfflineIndicator/OfflineIndicator";
import { adminNav } from "../navigation/registry";
import { ShellNav } from "./ShellNav";
import "./shell.css";

/**
 * Staff-only admin shell. Month 11 wires role checks (moderator+); until
 * then the route is gated by a placeholder that explains the requirement.
 */
export function AdminLayout() {
  return (
    <div className="shell shell--admin">
      <OfflineIndicator />
      <aside className="shell__sidebar shell__sidebar--admin">
        <Link to="/admin" className="shell__brand" aria-label="Admin home">
          <span className="shell__logo" aria-hidden="true">A</span>
          <span className="shell__brand-name">Admin</span>
        </Link>
        <ShellNav items={adminNav} />
        <div className="shell__sidebar-footer">
          <Link to="/" className="shell__link">
            Back to site
          </Link>
        </div>
      </aside>
      <div className="shell__body">
        <header className="shell__topbar">
          <div className="shell__topbar-title">Admin console</div>
        </header>
        <main className="shell__main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
