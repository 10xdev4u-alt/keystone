import { useState } from "react";
import "./App.css";
import { Avatar } from "./components/Avatar/Avatar";
import { Button } from "./components/Button/Button";
import { Dialog } from "./components/Dialog/Dialog";
import { Input } from "./components/Input/Input";
import { Select } from "./components/Select/Select";
import { EmptyState, ErrorState, Skeleton, Spinner } from "./components/Status/Status";
import { Tabs } from "./components/Tabs/Tabs";
import { ToastHost, type ToastData } from "./components/Toast/Toast";
import { useTheme } from "./theme";

const languageOptions = [
  { value: "rust", label: "Rust" },
  { value: "go", label: "Go" },
  { value: "typescript", label: "TypeScript" },
  { value: "python", label: "Python" },
];

const tabItems = [
  {
    value: "overview",
    label: "Overview",
    content: (
      <p>
        The design system is built from a spec: every component has variants, a11y tests, and
        empty/error/loading states.
      </p>
    ),
  },
  {
    value: "principles",
    label: "Principles",
    content: (
      <p>
        Accessible first — WCAG AA+, keyboard complete, reduced-motion aware. Tokens only, no
        hard-coded values in components.
      </p>
    ),
  },
];

export default function App() {
  const { mode, setMode } = useTheme();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [toasts, setToasts] = useState<ToastData[]>([]);

  const pushToast = (tone: "success" | "danger", title: string, description?: string) => {
    const id = crypto.randomUUID();
    setToasts((current) => [...current, { id, title, description, tone }]);
    window.setTimeout(() => setToasts((current) => current.filter((t) => t.id !== id)), 5000);
  };

  return (
    <div className="showcase">
      <header className="showcase__header">
        <div>
          <h1>Keystone Design System</h1>
          <p className="showcase__subtitle">
            Foundation components — Month 9. Built from a spec, not from vibes.
          </p>
        </div>
        <div className="showcase__tools">
          <Select
            label="Theme"
            value={mode}
            onValueChange={(value) => setMode(value as "light" | "dark" | "system")}
            options={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
          />
        </div>
      </header>

      <main className="showcase__main">
        <section className="card" aria-labelledby="h-buttons">
          <h2 id="h-buttons">Buttons</h2>
          <div className="row">
            <Button>Primary</Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="danger">Danger</Button>
            <Button loading>Working…</Button>
            <Button size="sm">Small</Button>
            <Button size="lg">Large</Button>
            <Button disabled>Disabled</Button>
          </div>
        </section>

        <section className="card" aria-labelledby="h-form">
          <h2 id="h-form">Form controls</h2>
          <div className="grid">
            <Input label="Email" hint="We never share it." type="email" placeholder="you@example.com" />
            <Input label="Password" error="Must be at least 12 characters." type="password" />
            <Select label="Language" options={languageOptions} placeholder="Pick a language…" />
          </div>
        </section>

        <section className="card" aria-labelledby="h-overlays">
          <h2 id="h-overlays">Overlays & feedback</h2>
          <div className="row">
            <Button onClick={() => setDialogOpen(true)}>Open dialog</Button>
            <Button onClick={() => pushToast("success", "Post published", "Your post is live.")}>
              Success toast
            </Button>
            <Button
              variant="danger"
              onClick={() => pushToast("danger", "Upload failed", "Quota exceeded (1 GiB).")}
            >
              Danger toast
            </Button>
          </div>
          <Dialog
            open={dialogOpen}
            onOpenChange={setDialogOpen}
            title="Delete post?"
            description="This cannot be undone."
            footer={
              <>
                <Button variant="ghost" onClick={() => setDialogOpen(false)}>
                  Cancel
                </Button>
                <Button variant="danger" onClick={() => setDialogOpen(false)}>
                  Delete
                </Button>
              </>
            }
          >
            <p>The post and all its comments will be permanently removed.</p>
          </Dialog>
        </section>

        <section className="card" aria-labelledby="h-tabs">
          <h2 id="h-tabs">Tabs</h2>
          <Tabs items={tabItems} />
        </section>

        <section className="card" aria-labelledby="h-status">
          <h2 id="h-status">Loading, empty & error</h2>
          <div className="grid">
            <div className="stack">
              <Skeleton className="skeleton--line" />
              <Skeleton className="skeleton--line" />
              <Skeleton className="skeleton--line skeleton--short" />
            </div>
            <EmptyState
              title="No posts yet"
              description="Be the first to write something the community will love."
              action={<Button variant="secondary">Write a post</Button>}
            />
            <ErrorState
              message="We couldn't reach the server. Check your connection."
              onRetry={() => pushToast("success", "Retrying…")}
            />
            <div className="stack">
              <Spinner label="Loading feed" />
            </div>
          </div>
        </section>

        <section className="card" aria-labelledby="h-avatar">
          <h2 id="h-avatar">Avatars</h2>
          <div className="row">
            <Avatar name="Barbara Liskov" size="sm" />
            <Avatar name="Alan Turing" />
            <Avatar name="Grace Hopper" size="lg" />
            <Avatar name="Linus Torvalds" src="https://i.pravatar.cc/64?img=7" />
          </div>
        </section>
      </main>

      <ToastHost toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((t) => t.id !== id))} />
    </div>
  );
}
