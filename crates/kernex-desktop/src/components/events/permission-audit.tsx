import { ShieldCheck, ShieldX } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { PermissionAudit } from "@/lib/types";

export function PermissionAuditItem({ audit }: { audit: PermissionAudit }) {
  const allowed = audit.decision.startsWith("allow");
  const Icon = allowed ? ShieldCheck : ShieldX;
  return <section className="content-auto flex items-start gap-3 rounded-md border bg-card/35 px-3 py-2.5">
    <Icon className={`mt-0.5 h-3.5 w-3.5 ${allowed ? "text-success" : "text-destructive"}`} />
    <div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><span className="text-xs font-medium">{audit.summary}</span><Badge variant={allowed ? "success" : "destructive"}>{audit.decision.replaceAll("_", " ")}</Badge><Badge variant="outline">{audit.risk}</Badge></div><p className="mt-1 break-all font-mono text-[10px] text-muted-foreground">{audit.resource}</p></div>
  </section>;
}
