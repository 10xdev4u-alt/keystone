import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// ── jsdom gaps Radix primitives depend on (jsdom has no layout engine) ─────

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver;

// Radix Select uses pointer capture + elementFromPoint for dismissable-layer
// detection; without these, popovers never open under jsdom.
Element.prototype.hasPointerCapture = () => false;
Element.prototype.setPointerCapture = () => {};
Element.prototype.releasePointerCapture = () => {};

// jsdom has no layout engine, so hit-testing is impossible; return the body
// as a stand-in. Radix uses this to decide whether a pointerdown happened
// outside a popover — body is "inside" everything, which keeps popovers open
// during tests.
document.elementFromPoint = () => document.body as unknown as Element;

// Radix Select also reads scroll position while positioning the popper, and
// calls `scrollIntoView` on the highlighted option after the content mounts.
// jsdom implements neither.
Object.defineProperty(window, "scrollX", { value: 0, configurable: true });
Object.defineProperty(window, "scrollY", { value: 0, configurable: true });
Element.prototype.scrollIntoView = () => {};

// Unmount React trees between tests so axe scans / queries never see
// leftover DOM from a previous test.
afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});
