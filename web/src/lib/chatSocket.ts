//! Live chat transport for the Messages UI.
//!
//! The backend socket (`/api/v1/ws/chat/{id}`) authenticates via the
//! `bearer.<jwt>` Sec-WebSocket-Protocol subprotocol because browsers cannot
//! set headers on a WebSocket upgrade. Frames mirror the Rust contract in
//! `crates/api/src/chat.rs`:
//!
//! ```json
//! { "type": "message" | "presence" | "typing" | "read" | "error",
//!   "conversation_id": "...",
//!   "payload": { ... } }
//! ```

import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { getAccessToken } from "../api/client";
import type { components } from "../api/generated";

type MessageListResponse = components["schemas"]["MessageListResponse"];
type MessageView = components["schemas"]["MessageView"];

export type PresenceStatus = "online" | "offline";

/** Client→server frame shapes (mirror `ClientFrame` in crates/api/src/chat.rs). */
export type ClientFrameOut =
  | { type: "message"; body: string }
  | { type: "typing" }
  | { type: "read" }
  | { type: "ack"; up_to?: string | null };

export type ChatFrame = {
  type: "message" | "presence" | "typing" | "read" | "error";
  conversation_id: string;
  payload: Record<string, unknown>;
};

/**
 * Apply one server frame to the React Query cache. Pure and exported for
 * direct unit testing; the hook is a thin socket shell around it.
 */
export function applyChatFrame(
  qc: QueryClient,
  conversationId: string,
  frame: ChatFrame,
): { presence?: Record<string, PresenceStatus>; typing?: string[] } {
  if (frame.conversation_id !== conversationId) return {};

  switch (frame.type) {
    case "message": {
      const p = frame.payload as unknown as Partial<MessageView>;
      if (typeof p.id !== "string" || typeof p.body !== "string") return {};
      const msg: MessageView = {
        id: p.id,
        conversation_id: conversationId,
        sender_id: p.sender_id ?? "",
        body: p.body,
        sent_at: p.sent_at ?? new Date().toISOString(),
        delivered_at: p.delivered_at ?? null,
        read_at: p.read_at ?? null,
      };
      qc.setQueryData<MessageListResponse>(
        ["conversations", conversationId, "messages"],
        (old) => {
          if (!old) return old;
          if (old.messages.some((m) => m.id === msg.id)) return old;
          return { ...old, messages: [...old.messages, msg] };
        },
      );
      return {};
    }
    case "presence": {
      const userId = frame.payload.user_id;
      const status = frame.payload.status;
      if (typeof userId === "string" && (status === "online" || status === "offline")) {
        return { presence: { [userId]: status } };
      }
      return {};
    }
    case "typing": {
      if (typeof frame.payload.user_id === "string") {
        return { typing: [frame.payload.user_id] };
      }
      return {};
    }
    default:
      return {};
  }
}

const TYPING_WINDOW_MS = 3000;
const MAX_BACKOFF_MS = 15_000;

/**
 * Open the live socket for one conversation. New messages are appended to the
 * messages cache automatically; presence and typing state are returned for the
 * thread header. Reconnects with exponential backoff; closes cleanly on
 * unmount or conversation switch.
 */
export function useChatSocket(conversationId: string | null) {
  const qc = useQueryClient();
  const [presence, setPresence] = useState<Record<string, PresenceStatus>>({});
  const [typing, setTyping] = useState<string[]>([]);
  const typingTimers = useRef(new Map<string, number>());
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!conversationId) return;
    const convId = conversationId;
    const token = getAccessToken();
    if (!token) return;

    let ws: WebSocket | null = null;
    let closed = false;
    let retries = 0;
    let timer: number | undefined;

    wsRef.current = null;

    const wsBase = (import.meta.env.VITE_API_URL as string | undefined) ?? "";

    function connect() {
      if (closed) return;
      ws = new WebSocket(`${wsBase}/api/v1/ws/chat/${convId}`, [`bearer.${token}`]);
      wsRef.current = ws;
      ws.onopen = () => {
        retries = 0;
      };
      ws.onmessage = (ev: MessageEvent<string>) => {
        let frame: ChatFrame;
        try {
          frame = JSON.parse(ev.data) as ChatFrame;
        } catch {
          return; // malformed frame — ignore
        }
        const update = applyChatFrame(qc, convId, frame);
        if (update.presence) {
          setPresence((prev) => ({ ...prev, ...update.presence! }));
        }
        if (update.typing) {
          const userId = update.typing[0];
          const existing = typingTimers.current.get(userId);
          if (existing) window.clearTimeout(existing);
          const t = window.setTimeout(() => {
            setTyping((prev) => prev.filter((u) => u !== userId));
            typingTimers.current.delete(userId);
          }, TYPING_WINDOW_MS);
          typingTimers.current.set(userId, t);
          setTyping((prev) => (prev.includes(userId) ? prev : [...prev, userId]));
        }
      };
      ws.onclose = () => {
        if (!closed) {
          const delay = Math.min(1000 * 2 ** retries, MAX_BACKOFF_MS);
          retries += 1;
          timer = window.setTimeout(connect, delay);
        }
      };
      ws.onerror = () => {
        ws?.close();
      };
    }

    connect();
    return () => {
      closed = true;
      if (timer) window.clearTimeout(timer);
      typingTimers.current.forEach((t) => window.clearTimeout(t));
      typingTimers.current.clear();
      ws?.close();
      wsRef.current = null;
    };
  }, [conversationId, qc]);

  /** Send one client frame if the socket is open (no-op otherwise). */
  const sendFrame = useCallback((frame: ClientFrameOut) => {
    const ws = wsRef.current;
    if (ws && typeof WebSocket !== "undefined" && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(frame));
    }
  }, []);

  return { presence, typing, sendFrame };
}
