import type { LucideIcon } from "lucide-react";

export function adsIcon(Icon: LucideIcon) {
  const Adapted = ({ label }: { label: string }) => (
    <Icon size={16} color="currentColor" aria-hidden={label ? undefined : true} aria-label={label || undefined} role={label ? "img" : undefined} />
  );
  Adapted.displayName = `adsIcon(${Icon.displayName ?? "Icon"})`;
  return Adapted;
}
