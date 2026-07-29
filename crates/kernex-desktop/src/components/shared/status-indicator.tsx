import { cn } from "@/lib/utils";

export function StatusIndicator({ status, label, className }: { status: "idle" | "running" | "success" | "warning" | "error" | "offline"; label: string; className?: string }) {
  const color = status === "success" ? "bg-success" : status === "warning" ? "bg-warning" : status === "error" ? "bg-destructive" : status === "running" ? "bg-foreground animate-pulse" : "bg-muted-foreground/60";
  return <span className={cn("inline-flex items-center gap-1.5 text-[10px] text-muted-foreground", className)}><span aria-hidden="true" className={cn("h-1.5 w-1.5 rounded-full", color)} /><span>{label}</span></span>;
}
