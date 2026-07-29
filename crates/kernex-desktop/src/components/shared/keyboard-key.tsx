import { cn } from "@/lib/utils";

export function KeyboardKey({ children, className }: { children: React.ReactNode; className?: string }) {
  return <kbd className={cn("inline-flex min-w-5 items-center justify-center rounded border bg-muted/60 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground shadow-[inset_0_-1px_0_var(--border)]", className)}>{children}</kbd>;
}
