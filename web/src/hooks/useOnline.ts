import { useEffect, useState } from "react";

/**
 * Live online/offline state. `navigator.onLine` is unreliable as a snapshot —
 * the events are the source of truth, and we listen for both window and
 * document (Safari fires on document).
 */
export function useOnline(): boolean {
  const [online, setOnline] = useState(() =>
    typeof navigator === "undefined" ? true : navigator.onLine,
  );

  useEffect(() => {
    const goOnline = () => setOnline(true);
    const goOffline = () => setOnline(false);
    window.addEventListener("online", goOnline);
    window.addEventListener("offline", goOffline);
    document.addEventListener("online", goOnline);
    document.addEventListener("offline", goOffline);
    return () => {
      window.removeEventListener("online", goOnline);
      window.removeEventListener("offline", goOffline);
      document.removeEventListener("online", goOnline);
      document.removeEventListener("offline", goOffline);
    };
  }, []);

  return online;
}
