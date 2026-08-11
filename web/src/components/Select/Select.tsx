import * as RadixSelect from "@radix-ui/react-select";
import { cn } from "../../lib/cn";
import { forwardRef, type ReactNode } from "react";
import "./select.css";

export interface SelectOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps {
  label: string;
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  error?: string;
  disabled?: boolean;
}

/**
 * Accessible select built on Radix. The trigger carries the full option
 * disclosure (`aria-haspopup`, expanded state, keyboard navigation) — the
 * visible label is hidden from the a11y tree so the trigger name is unique.
 */
export const Select = forwardRef<HTMLButtonElement, SelectProps>(
  (
    {
      label,
      value,
      defaultValue,
      onValueChange,
      options,
      placeholder = "Select…",
      error,
      disabled,
    },
    ref,
  ) => (
    <div className={cn("select", error && "select--invalid")}>
      <RadixSelect.Root
        value={value}
        defaultValue={defaultValue}
        onValueChange={onValueChange}
        disabled={disabled}
      >
        <RadixSelect.Trigger ref={ref} className="select__trigger" aria-label={label}>
          <RadixSelect.Value placeholder={placeholder} />
          <RadixSelect.Icon className="select__icon" aria-hidden="true">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path
                d="M2.5 4.5 6 8l3.5-3.5"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </RadixSelect.Icon>
        </RadixSelect.Trigger>
        <RadixSelect.Portal>
          <RadixSelect.Content className="select__content" position="popper" sideOffset={4}>
            <RadixSelect.Viewport>
              {options.map((option) => (
                <RadixSelect.Item
                  key={option.value}
                  value={option.value}
                  disabled={option.disabled}
                  className="select__item"
                >
                  <RadixSelect.ItemText>{option.label}</RadixSelect.ItemText>
                  <RadixSelect.ItemIndicator className="select__indicator">
                    ✓
                  </RadixSelect.ItemIndicator>
                </RadixSelect.Item>
              ))}
            </RadixSelect.Viewport>
          </RadixSelect.Content>
        </RadixSelect.Portal>
      </RadixSelect.Root>
      {error && (
        <p className="select__error" role="alert">
          {error}
        </p>
      )}
    </div>
  ),
);
Select.displayName = "Select";

export type { ReactNode };
