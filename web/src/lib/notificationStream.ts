//! Live notification stream for the app shell.
//!
//! The backend exposes `/api/v1/notifications/feed` as Server-Sent Events with
//! DB-backed gap recovery: send `Last-Event-ID: <notification-id>` on connect
//! and everything newer is replayed before the live stream chains on.
//! Browsers' native `EventSource` cannot send the `Authorization` header, so
//! we read the stream with `fetch` + a `ReadableStream` — the access token
//! stays in the header (never in a query string), and reconnects carry the
//! `Last-Event-ID` cursor automatically. Frame format mirrors the Rust handler
//! in `crates/api/src/realtime.rs`.

import { useEffect, useRef, useState } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { getAccessToken } from "../api/client";
import type { components } from "../api/generated";

type UnreadCountResponse = components["schemas"]["UnreadCountResponse"];

export type FeedEvent = {
  id: number;
  kind: string;
  payload: unknown;
};

const UNREAD_KEY = ["notifications", "unread"];
const LIST_KEY = ["notifications"];

/**
 * Apply one SSE notification event to the React Query caches. The badge bumps
 * instantly; the list is DB-backed, so invalidating it makes the next read
 * refetch and show the new item. Pure and exported for unit testing.
 */
export function applyFeedEvent(qc: QueryClient, _event: FeedEvent): void {
  qc.setQueryData<UnreadCountResponse>(UNREAD_KEY, (old) => ({
    unread: (old?.unread ?? 0) + 1,
  }));
  void qc.invalidateQueries({ queryKey: LIST_KEY });
}

/** Parse one SSE event block (`id:` / `event:` / `data:` lines). */
export function parseSseBlock(block: string): { id?: number; kind?: string; data?: string } {
  let id: number | undefined;
  let kind: string | undefined;
  let data: string | undefined;
  for (const line of block.split("\n")) {
    if (line.startsWith("id:")) {
      id = Number(line.slice(3).trim());
    } else if (line.startsWith("event:")) {
      kind = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      data = data === undefined ? line.slice(5).trim() : `${data}\n${line.slice(5).trim()}`;
    }
  }
  return { id, kind, data };
}

const MAX_BACKOFF_MS = 15_000;

/**
 * Open the live notification feed for the signed-in user and keep the unread
 * badge + notifications list current. Reconnects with exponential backoff and
 * re-sends `Last-Event-ID` so the server replays anything missed. Closes
 * cleanly on unmount; gives up on 401 (the in-memory token is gone).
 */
export function useNotificationStream(): { connected: boolean } {
  const qc = useQueryClient();
  const [connected, setConnected] = useState(false);
  const lastId = useRef<number | null>(null);

  useEffect(() => {
    const token = getAccessToken();
    if (!token) return;

    const controller = new AbortController();
    let closed = false;
    let retries = 0;
    let timer: number | undefined;
    let buffer = "";

    const apiBase = (import.meta.env.VITE_API_URL as string | undefined) ?? "";

    function handleBlock(block: string) {
      const parsed = parseSseBlock(block);
      if (parsed.id != null) lastId.current = parsed.id;
      if (parsed.kind === "resync") {
        // Channel overrun — the server asks for a DB replay.
        void qc.invalidateQueries({ queryKey: UNREAD_KEY });
        void qc.invalidateQueries({ queryKey: LIST_KEY });
        return;
      }
      if (parsed.id == null || parsed.data === undefined) return;
      let payload: unknown;
      try {
        payload = JSON.parse(parsed.data);
      } catch {
        return; // malformed data line — ignore
      }
      applyFeedEvent(qc, { id: parsed.id, kind: parsed.kind ?? "", payload });
    }

    async function connect() {
      if (closed) return;
      try {
        const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
        if (lastId.current != null) {
          headers["Last-Event-ID"] = String(lastId.current);
        }
        const res = await fetch(`${apiBase}/api/v1/notifications/feed`, {
          headers,
          signal: controller.signal,
        });
        if (!res.ok || !res.body) {
          if (res.status === 401) {
            closed = true; // token rotated/gone — stop, don't hammer
            setConnected(false);
            return;
          }
          throw new Error(`feed status ${res.status}`);
        }
        setConnected(true);
        retries = 0;
        buffer = "";
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let sep = buffer.indexOf("\n\n");
          while (sep !== -1) {
            handleBlock(buffer.slice(0, sep));
            buffer = buffer.slice(sep + 2);
            sep = buffer.indexOf("\n\n");
          }
        }
        setConnected(false);
      } catch (err) {
        if (closed || (err instanceof DOMException && err.name === "AbortError")) return;
        setConnected(false);
      }
      if (!closed) {
        const delay = Math.min(1000 * 2 ** retries, MAX_BACKOFF_MS);
        retries += 1;
        timer = window.setTimeout(() => void connect(), delay);
      }
    }

    void connect();
    return () => {
      closed = true;
      if (timer) window.clearTimeout(timer);
      controller.abort();
    };
  }, [qc]);

  return { connected };
}
