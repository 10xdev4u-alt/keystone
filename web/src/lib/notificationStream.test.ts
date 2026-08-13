import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { applyFeedEvent, parseSseBlock } from "./notificationStream";

function freshClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

describe("parseSseBlock", () => {
  it("parses id, event and data lines", () => {
    const parsed = parseSseBlock("id: 42\nevent: follow\ndata: {\"actor\":\"u2\"}");
    expect(parsed).toEqual({ id: 42, kind: "follow", data: '{"actor":"u2"}' });
  });

  it("joins multi-line data payloads", () => {
    const parsed = parseSseBlock("id: 7\nevent: comment\ndata: {\"a\":1,\ndata: \"b\":2}");
    expect(parsed.data).toBe('{"a":1,\n"b":2}');
  });

  it("returns empty for keep-alive comment blocks", () => {
    expect(parseSseBlock(": keep-alive")).toEqual({});
  });
});

describe("applyFeedEvent", () => {
  it("bumps the unread badge cache", () => {
    const qc = freshClient();
    qc.setQueryData(["notifications", "unread"], { unread: 3 });
    applyFeedEvent(qc, { id: 42, kind: "follow", payload: {} });
    expect(qc.getQueryData(["notifications", "unread"])).toEqual({ unread: 4 });
  });

  it("starts from zero when no unread cache exists", () => {
    const qc = freshClient();
    applyFeedEvent(qc, { id: 1, kind: "comment", payload: {} });
    expect(qc.getQueryData(["notifications", "unread"])).toEqual({ unread: 1 });
  });

  it("invalidates the DB-backed notifications list", () => {
    const qc = freshClient();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    applyFeedEvent(qc, { id: 1, kind: "comment", payload: {} });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["notifications"] });
  });
});
