import { lazy, Suspense, type ReactNode } from "react";
import type { RouteObject } from "react-router-dom";
import { Skeleton } from "../components/Status/Status";
import { AdminLayout } from "../layouts/AdminLayout";
import { AppLayout } from "../layouts/AppLayout";
import { PublicLayout } from "../layouts/PublicLayout";
import { NotFound } from "./NotFound";
import { RouteErrorBoundary } from "./ErrorBoundary";

// Lazy: every route loads its module on first navigation, so the initial
// bundle stays small and each screen splits into its own chunk. The
// placeholder module is one chunk today; real screens swap in per-file.
const Placeholder = lazy(() =>
  import("../pages/PlaceholderPage").then((m) => ({
    default: ({ title }: { title: string }) => <m.PlaceholderPage title={title} />,
  })),
);

const HomePage = lazy(() =>
  import("../pages/HomePage").then((m) => ({ default: m.HomePage })),
);
const PostPage = lazy(() =>
  import("../pages/PostPage").then((m) => ({ default: m.PostPage })),
);
const CommunitiesPage = lazy(() =>
  import("../pages/CommunitiesPage").then((m) => ({ default: m.CommunitiesPage })),
);
const CommunityPage = lazy(() =>
  import("../pages/CommunityPage").then((m) => ({ default: m.CommunityPage })),
);
const EventsPage = lazy(() =>
  import("../pages/EventsPage").then((m) => ({ default: m.EventsPage })),
);
const OrgsPage = lazy(() =>
  import("../pages/OrgsPage").then((m) => ({ default: m.OrgsPage })),
);
const OrgPage = lazy(() =>
  import("../pages/OrgPage").then((m) => ({ default: m.OrgPage })),
);
const SearchPage = lazy(() =>
  import("../pages/SearchPage").then((m) => ({ default: m.SearchPage })),
);
const ProfilePage = lazy(() =>
  import("../pages/ProfilePage").then((m) => ({ default: m.ProfilePage })),
);
const LoginPage = lazy(() =>
  import("../pages/auth/LoginPage").then((m) => ({ default: m.LoginPage })),
);
const RegisterPage = lazy(() =>
  import("../pages/auth/RegisterPage").then((m) => ({ default: m.RegisterPage })),
);
const VerifyPage = lazy(() =>
  import("../pages/auth/VerifyPage").then((m) => ({ default: m.VerifyPage })),
);

function withSuspense(node: ReactNode): ReactNode {
  return (
    <Suspense
      fallback={
        <div className="route-loading" data-testid="route-loading">
          <Skeleton className="route-loading__line" />
          <Skeleton className="route-loading__line" />
          <Skeleton className="route-loading__line route-loading__line--short" />
        </div>
      }
    >
      {node}
    </Suspense>
  );
}

/** Route map for the top-20 surface — one place to see the whole sitemap. */
export const routesConfig: RouteObject[] = [
  {
    element: <PublicLayout />,
    errorElement: <RouteErrorBoundary />,
    children: [
      { path: "/", element: withSuspense(<HomePage />) },
      { path: "/posts", element: withSuspense(<HomePage />) },
      { path: "/posts/:id", element: withSuspense(<PostPage />) },
      { path: "/communities", element: withSuspense(<CommunitiesPage />) },
      { path: "/communities/:slug", element: withSuspense(<CommunityPage />) },
      { path: "/events", element: withSuspense(<EventsPage />) },
      { path: "/events/:slug", element: withSuspense(<Placeholder title="Event" />) },
      { path: "/orgs", element: withSuspense(<OrgsPage />) },
      { path: "/orgs/:slug", element: withSuspense(<OrgPage />) },
      { path: "/search", element: withSuspense(<SearchPage />) },
      { path: "/users/:userId", element: withSuspense(<ProfilePage />) },
      { path: "/courses", element: withSuspense(<Placeholder title="Courses" />) },
      { path: "/login", element: withSuspense(<LoginPage />) },
      { path: "/register", element: withSuspense(<RegisterPage />) },
      { path: "/verify", element: withSuspense(<VerifyPage />) },
      { path: "*", element: <NotFound /> },
    ],
  },
  {
    element: <AppLayout />,
    errorElement: <RouteErrorBoundary />,
    children: [
      { path: "/me", element: withSuspense(<Placeholder title="My feed" />) },
      { path: "/me/notifications", element: withSuspense(<Placeholder title="Notifications" />) },
      { path: "/me/conversations", element: withSuspense(<Placeholder title="Messages" />) },
      { path: "/me/files", element: withSuspense(<Placeholder title="My files" />) },
      { path: "/me/profile", element: withSuspense(<Placeholder title="Profile" />) },
      { path: "/me/settings", element: withSuspense(<Placeholder title="Settings" />) },
    ],
  },
  {
    element: <AdminLayout />,
    errorElement: <RouteErrorBoundary />,
    children: [
      { path: "/admin", element: withSuspense(<Placeholder title="Admin overview" />) },
      { path: "/admin/moderation", element: withSuspense(<Placeholder title="Moderation queue" />) },
      { path: "/admin/users", element: withSuspense(<Placeholder title="Users" />) },
    ],
  },
];

// Router construction lives in Router.tsx — this module stays a pure route
// map so tests can build memory routers from the same config.
export type { RouteObject } from "react-router-dom";
