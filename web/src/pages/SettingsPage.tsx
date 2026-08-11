import { useEffect, useState, type FormEvent } from "react";
import { Link } from "react-router-dom";
import {
  useChangePassword,
  useCurrentUser,
  useNotificationPreferences,
  useProfile,
  useUpdateNotificationPreferences,
  useUpdateProfile,
} from "../api/hooks";
import type { components } from "../api/generated";

type SetProfileRequest = components["schemas"]["SetProfileRequest"];
import { Tabs } from "../components/Tabs/Tabs";
import { ErrorState, Spinner } from "../components/Status/Status";
import "./settings.css";

function ProfileTab() {
  const { data: me } = useCurrentUser();
  const { data, isLoading, isError, error } = useProfile(me?.id ?? "");
  const updateProfile = useUpdateProfile();
  const [bio, setBio] = useState("");
  const [location, setLocation] = useState("");
  const [visibility, setVisibility] = useState("public");
  const [loaded, setLoaded] = useState(false);

  if (!me) {
    return <ErrorState title="Sign in required" message="Sign in to edit your profile." />;
  }
  if (isLoading) return <Spinner label="Loading profile" />;
  if (isError || !data) {
    return <ErrorState title="Profile unavailable" message={error instanceof Error ? error.message : "Try again."} />;
  }

  // Hydrate the form once the profile arrives (keeps typing state while saving).
  useEffect(() => {
    if (!loaded && data) {
      setBio(data.profile.bio ?? "");
      setLocation(data.profile.location ?? "");
      setVisibility(data.profile.visibility);
      setLoaded(true);
    }
  }, [data, loaded]);

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    const body: SetProfileRequest = {
      bio: bio.trim() || null,
      location: location.trim() || null,
      visibility,
    };
    updateProfile.mutate(body);
  }

  return (
    <form className="settings-form" onSubmit={onSubmit}>
      <label className="settings-field">
        <span>Bio</span>
        <textarea
          value={bio}
          onChange={(e) => setBio(e.target.value)}
          rows={4}
          maxLength={4000}
          placeholder="Tell the community about yourself…"
        />
      </label>
      <label className="settings-field">
        <span>Location</span>
        <input
          value={location}
          onChange={(e) => setLocation(e.target.value)}
          maxLength={100}
          placeholder="City, country"
        />
      </label>
      <label className="settings-field">
        <span>Visibility</span>
        <select value={visibility} onChange={(e) => setVisibility(e.target.value)}>
          <option value="public">Public — everyone can see</option>
          <option value="connections">Connections — accepted connections only</option>
          <option value="private">Private — only you</option>
        </select>
      </label>
      {updateProfile.error && (
        <p className="settings-form__error">{updateProfile.error.message}</p>
      )}
      <div className="settings-form__actions">
        <button type="submit" className="btn btn--primary" disabled={updateProfile.isPending}>
          {updateProfile.isPending ? "Saving…" : "Save profile"}
        </button>
      </div>
    </form>
  );
}

function SecurityTab() {
  const changePassword = useChangePassword();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [done, setDone] = useState(false);

  const mismatch = confirm.length > 0 && next !== confirm;
  const error = changePassword.error?.message ?? null;

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (mismatch) return;
    changePassword.mutate(
      { current_password: current, new_password: next },
      {
        onSuccess: () => {
          setCurrent("");
          setNext("");
          setConfirm("");
          setDone(true);
        },
      },
    );
  }

  return (
    <div className="settings-security">
      <div className="settings-security__sessions">
        <h3>Active sessions</h3>
        <p>See every device signed into your account and revoke anything you don't recognize.</p>
        <Link to="/me/sessions" className="btn btn--secondary btn--sm">
          Manage sessions
        </Link>
      </div>

      <form className="settings-form" onSubmit={onSubmit}>
        <h3>Change password</h3>
        <p className="settings-form__hint">
          Changing your password signs out every other device.
        </p>
        <label className="settings-field">
          <span>Current password</span>
          <input
            type="password"
            autoComplete="current-password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            required
          />
        </label>
        <label className="settings-field">
          <span>New password</span>
          <input
            type="password"
            autoComplete="new-password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            required
          />
        </label>
        <label className="settings-field">
          <span>Confirm new password</span>
          <input
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
          />
        </label>
        {mismatch && <p className="settings-form__error">Passwords do not match.</p>}
        {error && <p className="settings-form__error">{error}</p>}
        {done && (
          <p className="settings-form__success" role="status">
            Password changed.
          </p>
        )}
        <div className="settings-form__actions">
          <button
            type="submit"
            className="btn btn--primary"
            disabled={changePassword.isPending || mismatch}
          >
            {changePassword.isPending ? "Changing…" : "Change password"}
          </button>
        </div>
      </form>
    </div>
  );
}

function NotificationsTab() {
  const { data, isLoading, isError, error, refetch } = useNotificationPreferences();
  const update = useUpdateNotificationPreferences();
  const [toggles, setToggles] = useState<{ in_app: boolean; digest: boolean; email: boolean } | null>(null);

  if (isLoading) return <Spinner label="Loading preferences" />;
  if (isError || !data) {
    return (
      <ErrorState
        title="Couldn't load preferences"
        message={error instanceof Error ? error.message : "Try again."}
        onRetry={() => void refetch()}
      />
    );
  }

  const prefs = toggles ?? {
    in_app: data.preferences.in_app,
    digest: data.preferences.digest,
    email: data.preferences.email,
  };

  function set(key: "in_app" | "digest" | "email", value: boolean) {
    const nextPrefs = { ...prefs, [key]: value };
    setToggles(nextPrefs);
    update.mutate(nextPrefs);
  }

  const rows: Array<{ key: "in_app" | "digest" | "email"; label: string; hint: string }> = [
    { key: "in_app", label: "In-app notifications", hint: "Banners and the notification bell." },
    { key: "email", label: "Email notifications", hint: "Important account and activity emails." },
    { key: "digest", label: "Weekly digest", hint: "A summary of what you missed." },
  ];

  return (
    <div className="settings-notifications">
      {rows.map((row) => (
        <label key={row.key} className="settings-switch">
          <span>
            <strong>{row.label}</strong>
            <em>{row.hint}</em>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={prefs[row.key]}
            onChange={(e) => set(row.key, e.target.checked)}
          />
        </label>
      ))}
      {update.error && <p className="settings-form__error">{update.error.message}</p>}
    </div>
  );
}

/** Account settings: profile, security, notifications. */
export function SettingsPage() {
  return (
    <div className="settings">
      <header className="settings__header">
        <h1 className="settings__title">Settings</h1>
        <p className="settings__subtitle">
          Your profile, sign-in security, and notification preferences.
        </p>
      </header>
      <Tabs
        defaultValue="profile"
        items={[
          { value: "profile", label: "Profile", content: <ProfileTab /> },
          { value: "security", label: "Security", content: <SecurityTab /> },
          { value: "notifications", label: "Notifications", content: <NotificationsTab /> },
        ]}
      />
    </div>
  );
}
