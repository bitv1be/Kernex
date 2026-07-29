import type { LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";

export function EmptyState({ icon: Icon, title, description, action, secondary }: { icon: LucideIcon; title: string; description: string; action?: { label: string; onClick: () => void }; secondary?: { label: string; onClick: () => void } }) {
  return <div className="flex min-h-44 flex-col items-center justify-center px-6 py-10 text-center">
    <div className="mb-4 flex h-9 w-9 items-center justify-center rounded-md border bg-muted/30"><Icon className="h-4 w-4 text-muted-foreground" /></div>
    <h3 className="text-sm font-medium">{title}</h3>
    <p className="mt-1.5 max-w-sm text-xs leading-5 text-muted-foreground">{description}</p>
    {(action || secondary) && <div className="mt-4 flex gap-2">{action && <Button size="sm" onClick={action.onClick}>{action.label}</Button>}{secondary && <Button size="sm" variant="outline" onClick={secondary.onClick}>{secondary.label}</Button>}</div>}
  </div>;
}
