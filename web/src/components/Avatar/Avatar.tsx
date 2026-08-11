import * as RadixAvatar from "@radix-ui/react-avatar";
import { cn } from "../../lib/cn";
import "./avatar.css";

export interface AvatarProps {
  /** Full name or handle used to derive initials + alt text. */
  name: string;
  src?: string;
  size?: "sm" | "md" | "lg";
  className?: string;
}

/** Initials fallback keeps the component meaningful when no image loads. */
export function Avatar({ name, src, size = "md", className }: AvatarProps) {
  const initials = name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");
  return (
    <RadixAvatar.Root className={cn("avatar", `avatar--${size}`, className)}>
      <RadixAvatar.Image className="avatar__image" src={src} alt={name} />
      <RadixAvatar.Fallback className="avatar__fallback" delayMs={300}>
        {initials || "?"}
      </RadixAvatar.Fallback>
    </RadixAvatar.Root>
  );
}
