import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { expectNoViolations } from "../../lib/test-utils";
import { OfflineIndicator } from "./OfflineIndicator";

function setNavigatorOnline(online: boolean) {
  Object.defineProperty(window.navigator, "onLine", {
    value: online,
    configurable: true,
  });
}

afterEach(() => {
  setNavigatorOnline(true);
  vi.restoreAllMocks();
});

describe("OfflineIndicator", () => {
  it("shows nothing while online", () => {
    setNavigatorOnline(true);
    render(<OfflineIndicator />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("appears when the offline event fires", () => {
    setNavigatorOnline(true);
    render(<OfflineIndicator />);
    fireEvent(window, new Event("offline"));
    expect(screen.getByRole("status")).toHaveTextContent(/offline/i);
  });

  it("disappears when the online event fires", () => {
    setNavigatorOnline(false);
    render(<OfflineIndicator />);
    fireEvent(window, new Event("online"));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("passes WCAG A/AA while visible", async () => {
    setNavigatorOnline(false);
    const { container } = render(<OfflineIndicator />);
    fireEvent(window, new Event("offline"));
    await expectNoViolations(container);
  });
});
