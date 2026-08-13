import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyChatFrame, useChatSocket, type ChatFrame } from "./chatSocket";

// The socket hook needs a real access token; stub the client module so the
// hook opens a (fake) connection instead of no-op'ing.
vi.mock("../api/client", () => ({
  getAccessToken: () => "test-jwt-token",
}));

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  static OPEN = 1;
  readyState = FakeWebSocket.OPEN;
  sent: string[] = [];

  constructor(
    public url: string,
    public protocols?: string[],
  ) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {}
}

const CONV = "conv-1";

function freshClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

function seedMessages(qc: QueryClient, messages: unknown[]) {
  qc.setQueryData(["conversations", CONV, "messages"], { messages });
}

function readMessages(qc: QueryClient): Array<{ id: string; body: string }> {
  const data = qc.getQueryData(["conversations", CONV, "messages"]) as {
    messages: Array<{ id: string; body: string }>;
  };
  return data?.messages ?? [];
}

const baseMessage = {
  id: "m1",
  conversation_id: CONV,
  sender_id: "u1",
  body: "first",
  sent_at: "2026-08-13T09:00:00Z",
  delivered_at: null,
  read_at: null,
};

describe("applyChatFrame", () => {
  it("appends a live message to the conversation cache", () => {
    const qc = freshClient();
    seedMessages(qc, [baseMessage]);

    applyChatFrame(qc, CONV, {
      type: "message",
      conversation_id: CONV,
      payload: { id: "m2", sender_id: "u2", body: "hello live", sent_at: "2026-08-13T09:01:00Z" },
    } as ChatFrame);

    const messages = readMessages(qc);
    expect(messages).toHaveLength(2);
    expect(messages[1].body).toBe("hello live");
  });

  it("dedupes repeated deliveries of the same message", () => {
    const qc = freshClient();
    seedMessages(qc, [baseMessage]);

    applyChatFrame(qc, CONV, {
      type: "message",
      conversation_id: CONV,
      payload: { id: "m1", sender_id: "u1", body: "first", sent_at: "2026-08-13T09:00:00Z" },
    } as ChatFrame);

    expect(readMessages(qc)).toHaveLength(1);
  });

  it("ignores frames for other conversations", () => {
    const qc = freshClient();
    seedMessages(qc, [baseMessage]);

    const update = applyChatFrame(qc, CONV, {
      type: "message",
      conversation_id: "other-conv",
      payload: { id: "x", body: "nope" },
    } as ChatFrame);

    expect(readMessages(qc)).toHaveLength(1);
    expect(update).toEqual({});
  });

  it("reports presence and typing updates", () => {
    const qc = freshClient();

    const presence = applyChatFrame(qc, CONV, {
      type: "presence",
      conversation_id: CONV,
      payload: { user_id: "u2", status: "online" },
    } as ChatFrame);
    expect(presence.presence).toEqual({ u2: "online" });

    const typing = applyChatFrame(qc, CONV, {
      type: "typing",
      conversation_id: CONV,
      payload: { user_id: "u2" },
    } as ChatFrame);
    expect(typing.typing).toEqual(["u2"]);
  });
});

describe("useChatSocket send", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function wrapper(qc: QueryClient) {
    return ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );
  }

  it("opens with the bearer subprotocol and sends typed frames", () => {
    const qc = freshClient();
    const { result } = renderHook(() => useChatSocket("conv-1"), {
      wrapper: wrapper(qc),
    });

    expect(FakeWebSocket.instances).toHaveLength(1);
    expect(FakeWebSocket.instances[0].url).toContain("/api/v1/ws/chat/conv-1");
    expect(FakeWebSocket.instances[0].protocols).toEqual(["bearer.test-jwt-token"]);

    act(() => {
      result.current.sendFrame({ type: "typing" });
      result.current.sendFrame({ type: "read" });
      result.current.sendFrame({ type: "ack", up_to: "2026-08-13T09:00:00Z" });
    });

    expect(FakeWebSocket.instances[0].sent).toEqual([
      '{"type":"typing"}',
      '{"type":"read"}',
      '{"type":"ack","up_to":"2026-08-13T09:00:00Z"}',
    ]);
  });

  it("reconnects with the same conversation after a close", () => {
    const qc = freshClient();
    renderHook(() => useChatSocket("conv-2"), { wrapper: wrapper(qc) });

    const first = FakeWebSocket.instances[0];
    expect(first).toBeDefined();

    // Reconnect is scheduled with backoff (1s) — arm fake timers BEFORE the
    // close so the timer is created on the fake clock.
    vi.useFakeTimers();
    // The hook's onclose schedules a reconnect — simulate the server drop.
    first.readyState = 3; // CLOSED
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (first as any).onclose?.(new Event("close"));

    act(() => {
      vi.advanceTimersByTime(1100);
    });
    expect(FakeWebSocket.instances.length).toBeGreaterThanOrEqual(2);
    vi.useRealTimers();
  });
});
