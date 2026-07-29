import { ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { useAppStore } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function PermissionDialog() {
  const approval = useAppStore((state) => state.approval);
  const setApproval = useAppStore((state) => state.setApproval);
  const decide = async (decision: string) => {
    if (!approval) return;
    await api.respondPermission(approval.id, decision);
    setApproval(undefined);
  };
  return (
    <Dialog open={Boolean(approval)} onOpenChange={(open) => { if (!open) void decide("deny"); }}>
      <DialogContent>
        <DialogHeader>
          <div className="mb-2 flex h-10 w-10 items-center justify-center rounded-full bg-amber-500/15 text-amber-400"><ShieldAlert className="h-5 w-5" /></div>
          <DialogTitle>Permission required</DialogTitle>
          <DialogDescription>{approval?.request.summary}</DialogDescription>
        </DialogHeader>
        <div className="rounded-md border bg-muted/30 p-3 text-sm">
          <div className="mb-2 flex justify-between"><span className="text-muted-foreground">Risk</span><span className="uppercase text-amber-400">{approval?.request.risk}</span></div>
          <div className="break-all"><span className="text-muted-foreground">Resource: </span>{approval?.request.resource}</div>
          {approval?.request.details.map((detail) => <pre key={detail} className="mt-3 max-h-48 overflow-auto whitespace-pre-wrap rounded bg-black/30 p-2 text-xs">{detail}</pre>)}
        </div>
        <DialogFooter className="flex-wrap">
          <Button variant="ghost" onClick={() => void decide("deny")}>Deny</Button>
          <Button variant="outline" onClick={() => void decide("allow_once")}>Allow once</Button>
          <Button variant="secondary" onClick={() => void decide("allow_for_session")}>For session</Button>
          <Button onClick={() => void decide("allow_for_project")}>For project</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
