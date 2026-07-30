import { Braces } from "lucide-react";
import { cn } from "@/lib/utils";

export function KernexMark({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("flex h-6 w-6 shrink-0 items-center justify-center rounded border border-foreground/20 bg-foreground text-background", className)}><Braces className="h-3.5 w-3.5" strokeWidth={2.25} /></span>;
}
