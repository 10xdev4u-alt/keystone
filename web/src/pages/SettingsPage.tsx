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

  // Hydrate the form once the profile arrives (keeps typing state while
  // saving). Runs unconditionally — hooks must not sit behind conditionals;
  // the data guard keeps it a no-op until the profile exists.
  useEffect(() => {
    if (!loaded && data) {
      setBio(data.profile.bio ?? "");
      setLocation(data.profile.location ?? "");
      setVisibility(data.profile.visibility);
      setLoaded(true);
    }
  }, [data, loaded]);

  if (!me) {
    return <ErrorState title="Sign in required" message="Sign in to edit your profile." />;
  }
  if (isLoading) return <Spinner label="Loading profile" />;
  if (isError || !data) {
    return <ErrorState title="Profile unavailable" message={error instanceof Error ? error.message : "Try again."} />;
  }

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

type PrefsState = {
  in_app: boolean;
  digest: boolean;
  email: boolean;
  muted_kinds: string[];
  quiet_hours_start: number | null;
  quiet_hours_end: number | null;
};

/** Minutes since midnight (the API's quiet-hours unit) to "HH:MM". */
function toTime(minutes: number): string {
  const h = Math.floor(minutes / 60) % 24;
  const m = minutes % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

/** "HH:MM" to minutes since midnight (the API's quiet-hours unit). */
function toMinutes(time: string): number {
  const [h, m] = time.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

const QUIET_DEFAULTS = { start: "22:00", end: "07:00" };

/** Every kind the backend can emit, so users can mute by type. */
const KIND_ROWS: Array<{ kind: string; label: string }> = [
  { kind: "follow", label: "Follows" },
  { kind: "comment", label: "Comments" },
  { kind: "reaction", label: "Reactions" },
  { kind: "answer", label: "Answers" },
  { kind: "message", label: "Direct messages" },
];

function NotificationsTab() {
  const { data, isLoading, isError, error, refetch } = useNotificationPreferences();
  const update = useUpdateNotificationPreferences();
  const [draft, setDraft] = useState<PrefsState | null>(null);

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

  const prefs: PrefsState = draft ?? {
    in_app: data.preferences.in_app,
    digest: data.preferences.digest,
    email: data.preferences.email,
    muted_kinds: data.preferences.muted_kinds,
    quiet_hours_start: data.preferences.quiet_hours_start ?? null,
    quiet_hours_end: data.preferences.quiet_hours_end ?? null,
  };

  function set(key: "in_app" | "digest" | "email", value: boolean) {
    const nextPrefs = { ...prefs, [key]: value };
    setDraft(nextPrefs);
    update.mutate(nextPrefs);
  }

  function toggleMuted(kind: string, muted: boolean) {
    const kinds = new Set(prefs.muted_kinds);
    if (muted) kinds.add(kind);
    else kinds.delete(kind);
    const nextPrefs = { ...prefs, muted_kinds: [...kinds] };
    setDraft(nextPrefs);
    update.mutate(nextPrefs);
  }

  function setQuietHours(start: string | null, end: string | null) {
    const nextPrefs: PrefsState = {
      ...prefs,
      quiet_hours_start: start === null ? null : toMinutes(start),
      quiet_hours_end: end === null ? null : toMinutes(end),
    };
    setDraft(nextPrefs);
    update.mutate(nextPrefs);
  }

  const quietEnabled = prefs.quiet_hours_start != null || prefs.quiet_hours_end != null;
  const startTime = prefs.quiet_hours_start != null ? toTime(prefs.quiet_hours_start) : QUIET_DEFAULTS.start;
  const endTime = prefs.quiet_hours_end != null ? toTime(prefs.quiet_hours_end) : QUIET_DEFAULTS.end;

  const rows: Array<{ key: "in_app" | "digest" | "email"; label: string; hint: string }> = [
    { key: "in_app", label: "In-app notifications", hint: "Banners and the notification bell." },
    { key: "email", label: "Email notifications", hint: "Important account and activity emails." },
    { key: "digest", label: "Weekly digest", hint: "A summary of what you missed." },
  ];

  return (
    <div className="settings-notifications">
      <section className="settings-section">
        <h3>Delivery</h3>
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
      </section>

      <section className="settings-section">
        <h3>Quiet hours</h3>
        <p className="settings-form__hint">Pause notifications every day during this window.</p>
        <label className="settings-switch">
          <span>
            <strong>Enable quiet hours</strong>
            <em>No banners, emails, or digests during quiet hours.</em>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={quietEnabled}
            onChange={(e) => {
              if (e.target.checked) setQuietHours(QUIET_DEFAULTS.start, QUIET_DEFAULTS.end);
              else setQuietHours(null, null);
            }}
          />
        </label>
        {quietEnabled && (
          <div className="settings-quiet">
            <label className="settings-field">
              <span>Quiet from</span>
              <input
                type="time"
                value={startTime}
                onChange={(e) => setQuietHours(e.target.value, endTime)}
              />
            </label>
            <label className="settings-field">
              <span>Until</span>
              <input
                type="time"
                value={endTime}
                onChange={(e) => setQuietHours(startTime, e.target.value)}
              />
            </label>
          </div>
        )}
      </section>

      <section className="settings-section">
        <h3>Mute by type</h3>
        <p className="settings-form__hint">Turn off notification kinds you don't care about.</p>
        {KIND_ROWS.map((row) => (
          <label key={row.kind} className="settings-check">
            <input
              type="checkbox"
              checked={prefs.muted_kinds.includes(row.kind)}
              onChange={(e) => toggleMuted(row.kind, e.target.checked)}
            />
            <span>{row.label}</span>
          </label>
        ))}
      </section>

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
