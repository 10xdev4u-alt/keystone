/**
 * Navigation registry — the single source of truth for every shell's nav.
 * Screens add themselves here (label + path + optional role); the layout
 * shells render from this, so a nav change is one edit, not three.
 */
import type { ReactNode } from "react";

export type ShellKind = "public" | "app" | "admin";

export interface NavItem {
  label: string;
  path: string;
  /** Icon element — small inline SVGs only, aria-hidden. */
  icon?: ReactNode;
  /** Role-gated items are filtered for the current user in the shell. */
  roles?: string[];
  /** Exact match for active state (false = prefix match). */
  end?: boolean;
}

const HomeIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="m2 7 6-5 6 5v6.5a.5.5 0 0 1-.5.5h-4V9.5h-3v4.5h-4A.5.5 0 0 1 2 13.5V7Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);
const PostIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M3 1.5h10a.5.5 0 0 1 .5.5v12.5L10 11.5H3A.5.5 0 0 1 2.5 11V2A.5.5 0 0 1 3 1.5Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);
const CommunityIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <circle cx="5.5" cy="5.5" r="2.5" stroke="currentColor" strokeWidth="1.3" />
    <path d="M1.5 14c.6-2.6 2-4 4-4s3.4 1.4 4 4M10.5 3.2a2.5 2.5 0 0 1 0 4.6M11.5 10c1.9.3 3 1.6 3 4" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);
const EventIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <rect x="2" y="3" width="12" height="11" rx="1.5" stroke="currentColor" strokeWidth="1.3" />
    <path d="M2 6.5h12M5.5 1.5v3M10.5 1.5v3" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);
const OrgIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M3 14V4.5L8 2v12M8 14V7l5 2v5" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);
const SearchIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <circle cx="7" cy="7" r="4.5" stroke="currentColor" strokeWidth="1.3" />
    <path d="m10.5 10.5 3.5 3.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);
const BellIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M8 2a4 4 0 0 1 4 4c0 3 .8 4 1.5 5h-11C3.2 10 4 9 4 6a4 4 0 0 1 4-4ZM6.5 13.5a1.8 1.8 0 0 0 3 0" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);
const ChatIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h9A1.5 1.5 0 0 1 14 4.5v6a1.5 1.5 0 0 1-1.5 1.5H6l-3.5 3v-4.2A1.5 1.5 0 0 1 2 10.5v-6Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);
const ShieldIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <path d="M8 1.5 13.5 4v4.5c0 3.2-2.2 5.2-5.5 6-3.3-.8-5.5-2.8-5.5-6V4L8 1.5Z" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
  </svg>
);
const UserIcon = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
    <circle cx="8" cy="5" r="3" stroke="currentColor" strokeWidth="1.3" />
    <path d="M2.5 14c.7-3 2.8-4.5 5.5-4.5s4.8 1.5 5.5 4.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
);

/** Public shell nav — visible to everyone, no auth required. */
export const publicNav: NavItem[] = [
  { label: "Home", path: "/", icon: HomeIcon, end: true },
  { label: "Posts", path: "/posts", icon: PostIcon },
  { label: "Communities", path: "/communities", icon: CommunityIcon },
  { label: "Events", path: "/events", icon: EventIcon },
  { label: "Organizations", path: "/orgs", icon: OrgIcon },
  { label: "Search", path: "/search", icon: SearchIcon },
];

/** Authenticated app shell nav. */
export const appNav: NavItem[] = [
  { label: "My feed", path: "/me", icon: HomeIcon, end: true },
  { label: "Notifications", path: "/me/notifications", icon: BellIcon },
  { label: "Messages", path: "/me/conversations", icon: ChatIcon },
  { label: "Files", path: "/me/files", icon: PostIcon },
  { label: "Profile", path: "/me/profile", icon: UserIcon },
  { label: "Sessions", path: "/me/sessions", icon: ShieldIcon },
];

/** Staff-only admin shell nav. */
export const adminNav: NavItem[] = [
  { label: "Overview", path: "/admin", icon: ShieldIcon, end: true },
  { label: "Moderation", path: "/admin/moderation", icon: ShieldIcon },
  { label: "Users", path: "/admin/users", icon: UserIcon },
];

/** Look up the registry for a shell — keeps layouts dumb. */
export function navFor(shell: ShellKind): NavItem[] {
  switch (shell) {
    case "public":
      return publicNav;
    case "app":
      return appNav;
    case "admin":
      return adminNav;
  }
}
