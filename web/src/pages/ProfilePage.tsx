import { useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useCurrentUser, useProfile, useUpdateProfile } from "../api/hooks";
import { ErrorState, Skeleton } from "../components/Status/Status";
import type { components } from "../api/generated";
import "./profile.css";

type SetProfileRequest = components["schemas"]["SetProfileRequest"];

function VisibilityBadge({ visibility }: { visibility: string }) {
  const label =
    visibility === "public"
      ? "Public"
      : visibility === "connections"
        ? "Connections only"
        : "Private";
  return <span className={`badge badge--${visibility}`}>{label}</span>;
}

export function ProfilePage() {
  const { userId } = useParams<{ userId: string }>();
  const navigate = useNavigate();
  const { data: me } = useCurrentUser();
  const { data, isLoading, isError, error } = useProfile(userId ?? "");
  const [editing, setEditing] = useState(false);
  const updateProfile = useUpdateProfile({
    onSuccess: () => setEditing(false),
  });

  if (isLoading) {
    return (
      <div className="profile">
        <Skeleton className="profile__skeleton-title" />
        <Skeleton className="profile__skeleton-line" />
        <Skeleton className="profile__skeleton-line" />
        <Skeleton className="profile__skeleton-line" />
      </div>
    );
  }
  if (isError || !data) {
    return (
      <ErrorState
        title="Profile unavailable"
        message={
          error instanceof Error
            ? error.message
            : "This profile is hidden or doesn't exist."
        }
        onRetry={() => navigate("/")}
        retryLabel="Back to feed"
      />
    );
  }

  const isOwn = me?.id === userId;

  return (
    <div className="profile">
      <header className="profile__header">
        <div>
          <h1 className="profile__name">
            {me?.id === userId ? "Your profile" : "Member profile"}
          </h1>
          <p className="profile__meta">
            <VisibilityBadge visibility={data.profile.visibility} />
            {data.profile.location && (
              <span className="profile__location">📍 {data.profile.location}</span>
            )}
          </p>
        </div>
        {isOwn && !editing && (
          <button className="btn btn--secondary" onClick={() => setEditing(true)}>
            Edit profile
          </button>
        )}
      </header>

      {editing ? (
        <ProfileEditor
          initial={data.profile}
          submitting={updateProfile.isPending}
          error={updateProfile.error?.message}
          onSubmit={(body) => updateProfile.mutate(body)}
          onCancel={() => setEditing(false)}
        />
      ) : (
        <>
          <section className="profile__section">
            <h2>About</h2>
            <p className="profile__bio">
              {data.profile.bio || "This member hasn't written a bio yet."}
            </p>
          </section>

          <ProfileSection title="Education" items={data.education} empty="No education listed">
            {data.education.map((e) => (
              <div key={e.id} className="profile__entry">
                <h3>{e.school}</h3>
                <p className="profile__entry-sub">
                  {[e.degree, e.field].filter(Boolean).join(" · ")}
                </p>
                <p className="profile__entry-meta">
                  {e.start_year}
                  {e.end_year ? ` – ${e.end_year}` : " – present"}
                </p>
                {e.description && <p>{e.description}</p>}
              </div>
            ))}
          </ProfileSection>

          <ProfileSection title="Experience" items={data.experience} empty="No experience listed">
            {data.experience.map((x) => (
              <div key={x.id} className="profile__entry">
                <h3>{x.title}</h3>
                <p className="profile__entry-sub">
                  {[x.company, x.organization_id && "linked org"]
                    .filter(Boolean)
                    .join(" at ")}
                </p>
                <p className="profile__entry-meta">
                  {x.start_date}
                  {x.current || x.end_date ? ` – ${x.current ? "present" : x.end_date}` : ""}
                </p>
                {x.description && <p>{x.description}</p>}
              </div>
            ))}
          </ProfileSection>

          <ProfileSection title="Skills" items={data.skills} empty="No skills listed">
            <ul className="profile__skills">
              {data.skills.map((s) => (
                <li key={s.skill} className="skill-pill">
                  {s.skill}
                  {s.level !== "none" && <em>{s.level}</em>}
                </li>
              ))}
            </ul>
          </ProfileSection>
        </>
      )}
    </div>
  );
}

function ProfileSection<T>({
  title,
  items,
  empty,
  children,
}: {
  title: string;
  items: T[];
  empty: string;
  children: React.ReactNode;
}) {
  return (
    <section className="profile__section">
      <h2>{title}</h2>
      {items.length === 0 ? <p className="profile__empty">{empty}</p> : children}
    </section>
  );
}

function ProfileEditor({
  initial,
  submitting,
  error,
  onSubmit,
  onCancel,
}: {
  initial: { bio?: string | null; location?: string | null; visibility: string };
  submitting: boolean;
  error?: string;
  onSubmit: (body: SetProfileRequest) => void;
  onCancel: () => void;
}) {
  const [bio, setBio] = useState(initial.bio ?? "");
  const [location, setLocation] = useState(initial.location ?? "");
  const [visibility, setVisibility] = useState(initial.visibility);

  return (
    <form
      className="profile__editor"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit({
          bio: bio.trim() || null,
          location: location.trim() || null,
          visibility,
        });
      }}
    >
      <label>
        <span>Bio</span>
        <textarea
          value={bio}
          onChange={(e) => setBio(e.target.value)}
          rows={4}
          maxLength={4000}
          placeholder="Tell the community about yourself…"
        />
      </label>
      <label>
        <span>Location</span>
        <input
          value={location}
          onChange={(e) => setLocation(e.target.value)}
          maxLength={100}
          placeholder="City, country"
        />
      </label>
      <label>
        <span>Visibility</span>
        <select value={visibility} onChange={(e) => setVisibility(e.target.value)}>
          <option value="public">Public — everyone can see</option>
          <option value="connections">Connections — accepted connections only</option>
          <option value="private">Private — only you</option>
        </select>
      </label>
      {error && <p className="form-error">{error}</p>}
      <div className="profile__editor-actions">
        <button type="button" className="btn btn--ghost" onClick={onCancel}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary" disabled={submitting}>
          {submitting ? "Saving…" : "Save changes"}
        </button>
      </div>
    </form>
  );
}
