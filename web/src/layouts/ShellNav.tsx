import { NavLink } from "react-router-dom";
import { cn } from "../lib/cn";
import { type NavItem } from "../navigation/registry";

/**
 * Renders a nav item list from the registry with active-state styling.
 * `NavLink` sets aria-current="page" automatically on the active item.
 */
export function ShellNav({ items, className }: { items: NavItem[]; className?: string }) {
  return (
    <nav aria-label="Primary" className={cn("shell-nav", className)}>
      <ul>
        {items.map((item) => (
          <li key={item.path}>
            <NavLink
              to={item.path}
              end={item.end}
              className={({ isActive }) => cn("shell-nav__link", isActive && "shell-nav__link--active")}
            >
              {item.icon}
              <span>{item.label}</span>
            </NavLink>
          </li>
        ))}
      </ul>
    </nav>
  );
}
