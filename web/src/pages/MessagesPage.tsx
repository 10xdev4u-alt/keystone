import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  useConversations,
  useCreateConversation,
  useCurrentUser,
  useMessages,
  useSendMessage,
} from "../api/hooks";
import { ApiRequestError } from "../api/client";
import { Button } from "../components/Button/Button";
import { Input } from "../components/Input/Input";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import { useChatSocket } from "../lib/chatSocket";
import { cn } from "../lib/cn";
import "./messages.css";

function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function conversationLabel(type: string, title: string | null | undefined): string {
  if (title) return title;
  return type === "group" ? "Group chat" : "Direct message";
}

/** Messages: conversation list + thread, REST-backed (WS fan-out is a bonus). */
export function MessagesPage() {
  const { data: me } = useCurrentUser();
  const [activeId, setActiveId] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);
  const [newUserId, setNewUserId] = useState("");
  const [draft, setDraft] = useState("");
  const { data: convos, isLoading, isError, error, refetch } = useConversations({
    enabled: Boolean(me),
  });
  const { data: messages } = useMessages(activeId);
  const { presence, typing, sendFrame } = useChatSocket(activeId);
  const send = useSendMessage(activeId, {
    onSuccess: () => setDraft(""),
  });
  const typingSentAt = useRef(0);
  const createConv = useCreateConversation({
    onSuccess: (data) => {
      setActiveId(data.conversation.id);
      setShowNew(false);
      setNewUserId("");
    },
  });
  const threadEndRef = useRef<HTMLDivElement | null>(null);

  // Auto-scroll the thread to the newest message.
  useEffect(() => {
    threadEndRef.current?.scrollIntoView({ block: "end" });
  }, [messages?.messages.length, activeId]);

  // Mark the thread read when it opens or new messages arrive (the socket
  // tells the other participant we're reading).
  useEffect(() => {
    if (activeId && (messages?.messages.length ?? 0) > 0) {
      sendFrame({ type: "read" });
    }
  }, [activeId, messages?.messages.length, sendFrame]);

  // Emit typing frames while drafting, throttled to one per 2s.
  function onDraftChange(value: string) {
    setDraft(value);
    const now = Date.now();
    if (value.trim() && now - typingSentAt.current > 2000) {
      typingSentAt.current = now;
      sendFrame({ type: "typing" });
    }
  }

  function onSend(e: FormEvent) {
    e.preventDefault();
    if (!activeId || draft.trim().length === 0) return;
    send.mutate({ body: draft.trim() });
  }

  function onNewConversation(e: FormEvent) {
    e.preventDefault();
    const id = newUserId.trim();
    if (!id) return;
    createConv.mutate({ user_id: id });
  }

  const newError =
    createConv.error instanceof ApiRequestError
      ? createConv.error.detail ?? createConv.error.message
      : null;

  return (
    <div className="messages">
      <header className="messages__header">
        <h1 className="messages__title">Messages</h1>
        <Button variant="secondary" size="sm" onClick={() => setShowNew((s) => !s)}>
          {showNew ? "Cancel" : "New message"}
        </Button>
      </header>

      {showNew && (
        <form className="messages__new" onSubmit={onNewConversation}>
          <Input
            id="new-conversation-user"
            label="User ID"
            value={newUserId}
            onChange={(e) => setNewUserId(e.target.value)}
            placeholder="Paste the other user's id to start a direct conversation"
          />
          {newError && <p className="messages__error">{newError}</p>}
          <Button type="submit" size="sm" loading={createConv.isPending}>
            Start conversation
          </Button>
        </form>
      )}

      <div className="messages__layout">
        <aside className="messages__list" aria-label="Conversations">
          {isLoading ? (
            <div data-testid="conversations-loading" aria-label="Loading conversations">
              {[0, 1, 2].map((i) => (
                <div className="messages__convo-skeleton" key={i}>
                  <Skeleton className="messages__convo-line" />
                </div>
              ))}
            </div>
          ) : isError ? (
            <ErrorState
              title="Couldn't load conversations"
              message={error instanceof Error ? error.message : undefined}
              onRetry={() => void refetch()}
            />
          ) : (convos?.conversations ?? []).length === 0 ? (
            <EmptyState
              headingLevel={2}
              title="No conversations yet"
              description="Start a direct message or wait for someone to reach out."
            />
          ) : (
            <ul>
              {convos!.conversations.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    className={cn(
                      "messages__convo",
                      c.id === activeId && "messages__convo--active",
                    )}
                    onClick={() => setActiveId(c.id)}
                  >
                    <span className="messages__convo-title">
                      {conversationLabel(c.type, c.title)}
                    </span>
                    {c.last_message && (
                      <span className="messages__convo-preview">{c.last_message}</span>
                    )}
                    <span className="messages__convo-meta">
                      {c.last_message_at ? timeAgo(c.last_message_at) : "no messages yet"}
                      {c.unread > 0 && <span className="messages__convo-unread">{c.unread}</span>}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </aside>

        <section className="messages__thread" aria-label="Thread">
          {!activeId ? (
            <EmptyState
              headingLevel={2}
              title="Select a conversation"
              description="Pick a thread on the left to read and reply."
            />
          ) : (
            <>
              <header className="messages__thread-head">
                <h2 className="messages__thread-title">
                  {conversationLabel(
                    convos?.conversations.find((c) => c.id === activeId)?.type ?? "direct",
                    convos?.conversations.find((c) => c.id === activeId)?.title ?? null,
                  )}
                </h2>
                <p className="messages__thread-status" role="status">
                  {typing.some((u) => u !== me?.id)
                    ? "Typing…"
                    : Object.entries(presence).some(
                        ([id, status]) => status === "online" && id !== me?.id,
                      )
                      ? "Online"
                      : ""}
                </p>
              </header>
              <div className="messages__thread-body">
                {!messages ? (
                  <div data-testid="messages-loading" aria-label="Loading messages">
                    {[0, 1].map((i) => (
                      <div className="messages__msg-skeleton" key={i}>
                        <Skeleton className="messages__msg-line" />
                      </div>
                    ))}
                  </div>
                ) : messages.messages.length === 0 ? (
                  <EmptyState
                    headingLevel={3}
                    title="Say hello"
                    description="No messages yet — start the conversation."
                  />
                ) : (
                  messages.messages.map((m) => {
                    const mine = m.sender_id === me?.id;
                    return (
                      <div
                        key={m.id}
                        className={cn("messages__msg", mine && "messages__msg--mine")}
                      >
                        <p className="messages__msg-body">{m.body}</p>
                        <time className="messages__msg-time">{timeAgo(m.sent_at)}</time>
                      </div>
                    );
                  })
                )}
                <div ref={threadEndRef} />
              </div>

              <form className="messages__composer" onSubmit={onSend}>
                <input
                  id="message-body"
                  className="messages__input"
                  aria-label="Message"
                  value={draft}
                  onChange={(e) => onDraftChange(e.target.value)}
                  placeholder="Write a message…"
                  autoComplete="off"
                />
                <Button type="submit" size="sm" disabled={draft.trim().length === 0} loading={send.isPending}>
                  Send
                </Button>
              </form>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
