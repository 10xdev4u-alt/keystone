import { clsx, type ClassValue } from "clsx";

/**
 * Merge class names with `clsx`. The single allowed class-composition helper
 * in the design system — components accept `className` for layout overrides,
 * never for design tokens.
 */
export function cn(...inputs: ClassValue[]): string {
  return clsx(inputs);
}
