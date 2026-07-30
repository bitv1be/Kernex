import { FolderLock, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function PermissionDialog() {
  const approval = useAppStore((state) => state.approval);
  const setApproval = useAppStore((state) => state.setApproval);
  const setError = useAppStore((state) => state.setError);
  const decide = async (decision: string) => {
    if (!approval) return;
    try {
      await api.respondPermission(approval.id, decision);
      setApproval(undefined);
    } catch (error) {
      setError(`Could not record the permission decision: ${String(error)}`);
    }
  };
  return (
    <Dialog open={Boolean(approval)} onOpenChange={(open) => { if (!open) void decide("deny"); }}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <div className="mb-2 flex h-9 w-9 items-center justify-center rounded-md border bg-muted/35"><ShieldAlert className="h-4 w-4 text-warning" /></div>
          <div className="flex items-center gap-2"><DialogTitle>Permission required</DialogTitle><Badge variant="warning">{approval?.request.risk} risk</Badge></div>
          <DialogDescription>Kernex paused before a protected action. Review the exact scope before continuing.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3 rounded-md border bg-muted/20 p-4 text-sm">
          <div><div className="mb-1 text-[9px] font-medium uppercase tracking-wider text-muted-foreground">Requested action</div><p className="text-xs font-medium">{approval?.request.summary}</p></div>
          <div><div className="mb-1 flex items-center gap-1 text-[9px] font-medium uppercase tracking-wider text-muted-foreground"><FolderLock className="h-3 w-3" />Affected resource or working directory</div><p className="break-all font-mono text-[11px]">{approval?.request.resource}</p></div>
          <div><div className="mb-1 text-[9px] font-medium uppercase tracking-wider text-muted-foreground">Capability</div><Badge variant="outline">{approval?.request.capability.replaceAll("_", " ")}</Badge></div>
          {approval?.request.details.map((detail, index) => <div key={`${detail}-${index}`}><div className="mb-1 text-[9px] font-medium uppercase tracking-wider text-muted-foreground">Operation details</div><pre className="max-h-52 overflow-auto whitespace-pre-wrap break-all rounded border bg-terminal p-3 font-mono text-[10px] leading-5 text-zinc-200">{detail}</pre></div>)}
        </div>
        <DialogFooter className="flex-wrap">
          <Button variant="outline" onClick={() => void decide("deny")}>Deny</Button>
          <Button variant="outline" onClick={() => void decide("allow_once")}>Allow once</Button>
          <Button variant="secondary" onClick={() => void decide("allow_for_session")}>For session</Button>
          <Button onClick={() => void decide("allow_for_project")}>For project</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
