import { Link, Outlet } from "react-router-dom";
import { OfflineIndicator } from "../components/OfflineIndicator/OfflineIndicator";
import { publicNav } from "../navigation/registry";
import { ShellNav } from "./ShellNav";
import "./shell.css";

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
        <div className="shell__actions">
          <Link to="/login" className="shell__link">
            Sign in
          </Link>
          <Link to="/register" className="shell__link shell__link--cta">
            Join free
          </Link>
        </div>
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
