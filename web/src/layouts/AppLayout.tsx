import { Link, Outlet, useNavigate } from "react-router-dom";
import { useUnreadCount } from "../api/hooks";
import { useNotificationStream } from "../lib/notificationStream";
import { OfflineIndicator } from "../components/OfflineIndicator/OfflineIndicator";
import { Avatar } from "../components/Avatar/Avatar";
import { Button } from "../components/Button/Button";
import { appNav } from "../navigation/registry";
import { ShellNav } from "./ShellNav";
import "./shell.css";

/**
 * Authenticated app shell: sidebar nav + top bar + content.
 * Month 10 wires real auth state here (token in memory, redirect when absent).
 */
export function AppLayout() {
  const navigate = useNavigate();
  const { data: unread } = useUnreadCount({ retry: false });
  const { connected } = useNotificationStream();
  const nav = appNav.map((item) =>
    item.path === "/me/notifications" && unread?.unread
      ? { ...item, badge: unread.unread }
      : item,
  );
  return (
    <div className="shell shell--app">
      <OfflineIndicator />
      <aside className="shell__sidebar">
        <Link to="/me" className="shell__brand" aria-label="Keystone home">
          <span className="shell__logo" aria-hidden="true">K</span>
          <span className="shell__brand-name">Keystone</span>
        </Link>
        <ShellNav items={nav} />
        <div className="shell__sidebar-footer">
          <Button variant="ghost" size="sm" onClick={() => navigate("/")}>
            Back to public site
          </Button>
        </div>
      </aside>
      <div className="shell__body">
        <header className="shell__topbar">
          <div className="shell__topbar-title">
            Workspace
            {connected && (
              <span className="shell__live" title="Live notifications on">
                ● live
              </span>
            )}
          </div>
          <Avatar name="Current User" size="sm" />
        </header>
        <main className="shell__main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
