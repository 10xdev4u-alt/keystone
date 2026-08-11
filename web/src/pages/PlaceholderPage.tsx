import { EmptyState } from "../components/Status/Status";

/**
 * Month 10 replaces each placeholder with a real screen. Kept in ONE module
 * so the whole placeholder set is a single small chunk; real pages get their
 * own files (and their own chunks) as they land.
 */
export function PlaceholderPage({ title }: { title: string }) {
  return (
    <EmptyState
      title={title}
      description="This screen is on the Month 10 build-out list. The shell, nav and route are already live."
    />
  );
}
