import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { applyChatFrame, type ChatFrame } from "./chatSocket";

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
