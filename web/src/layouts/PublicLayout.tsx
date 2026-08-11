import { Link, Outlet, useNavigate } from "react-router-dom";
import { useCurrentUser, useLogout } from "../api/hooks";
import { Avatar } from "../components/Avatar/Avatar";
import { OfflineIndicator } from "../components/OfflineIndicator/OfflineIndicator";
import { publicNav } from "../navigation/registry";
import { ShellNav } from "./ShellNav";
import "./shell.css";

function AuthActions() {
  const navigate = useNavigate();
  const { data: me } = useCurrentUser();
  const logout = useLogout({
    onSuccess: () => navigate("/"),
  });

  if (me) {
    return (
      <div className="shell__actions shell__actions--user">
        <Link to="/me" className="shell__user">
          <Avatar name={me.username ?? me.email} size="sm" />
          <span className="shell__user-name">{me.username ?? me.email}</span>
        </Link>
        <Link to="/me/settings" className="shell__link">
          Settings
        </Link>
        <button
          type="button"
          className="shell__link shell__link--btn"
          onClick={() => logout.mutate()}
          disabled={logout.isPending}
        >
          {logout.isPending ? "Signing out…" : "Sign out"}
        </button>
      </div>
    );
  }

  return (
    <div className="shell__actions">
      <Link to="/login" className="shell__link">
        Sign in
      </Link>
      <Link to="/register" className="shell__link shell__link--cta">
        Join free
      </Link>
    </div>
  );
}

/** Public marketing/content shell: header nav + footer. */
export function PublicLayout() {
  return (
    <div className="shell">
      <OfflineIndicator />
      <header className="shell__header">
        <Link to="/" className="shell__brand" aria-label="Keystone home">
          <span className="shell__logo" aria-hidden="true">K</span>
          <span className="shell__brand-name">Keystone</span>
        </Link>
        <ShellNav items={publicNav} />
        <AuthActions />
      </header>
      <main className="shell__main">
        <Outlet />
      </main>
      <footer className="shell__footer">
        <p>Keystone — built from a spec, not from vibes.</p>
      </footer>
    </div>
  );
}
