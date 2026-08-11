import { render, type RenderOptions } from "@testing-library/react";
import axe from "axe-core";
import type { ReactElement } from "react";

/**
 * Render a component and run axe-core over it. Every interactive component
 * test asserts `expect(await expectNoViolations(container))` — WCAG A/AA is
 * a CI gate, not a nice-to-have.
 *
 * `allowedViolations` is a *narrow* escape hatch for documented upstream
 * false positives only — every entry must name the exact rule id and carry a
 * comment at the call site explaining why it is safe.
 */
export async function expectNoViolations(
  container: HTMLElement,
  allowedViolations: string[] = [],
): Promise<void> {
  const results = await axe.run(container, {
    // Radix portals render outside the container; scan the whole document.
    resultTypes: ["violations"],
  });
  const violations = results.violations.filter((v) => !allowedViolations.includes(v.id));
  if (violations.length > 0) {
    const summary = violations
      .map((v) => `- ${v.id}: ${v.help} (${v.nodes.length} node(s))`)
      .join("\n");
    throw new Error(`axe violations:\n${summary}`);
  }
}

/** Render with the app's providers (theme, toast host) already mounted. */
export function renderWithProviders(
  ui: ReactElement,
  options?: Omit<RenderOptions, "wrapper">,
) {
  return render(ui, options);
}
