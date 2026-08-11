import * as RadixTabs from "@radix-ui/react-tabs";
import { cn } from "../../lib/cn";
import { type ReactNode } from "react";
import "./tabs.css";

export interface TabItem {
  value: string;
  label: string;
  content: ReactNode;
}

export interface TabsProps {
  items: TabItem[];
  defaultValue?: string;
  value?: string;
  onValueChange?: (value: string) => void;
  /** Extra trigger labels, e.g. a count badge, keyed by tab value. */
  adornments?: Record<string, ReactNode>;
}

/** Tabs with full keyboard arrow-key navigation (Radix). */
export function Tabs({ items, defaultValue, value, onValueChange, adornments }: TabsProps) {
  return (
    <RadixTabs.Root
      className="tabs"
      defaultValue={defaultValue ?? items[0]?.value}
      value={value}
      onValueChange={onValueChange}
    >
      <RadixTabs.List className="tabs__list" aria-label="Content sections">
        {items.map((item) => (
          <RadixTabs.Trigger key={item.value} value={item.value} className="tabs__trigger">
            {item.label}
            {adornments?.[item.value]}
          </RadixTabs.Trigger>
        ))}
      </RadixTabs.List>
      {items.map((item) => (
        <RadixTabs.Content key={item.value} value={item.value} className={cn("tabs__content")}>
          {item.content}
        </RadixTabs.Content>
      ))}
    </RadixTabs.Root>
  );
}
