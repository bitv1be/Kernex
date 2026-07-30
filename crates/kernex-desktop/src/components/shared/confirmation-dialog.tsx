import { TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";

export function ConfirmationDialog({ open, title, description, confirmLabel, destructive = false, onOpenChange, onConfirm }: { open: boolean; title: string; description: string; confirmLabel: string; destructive?: boolean; onOpenChange: (open: boolean) => void; onConfirm: () => void }) {
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="max-w-md"><DialogHeader><div className="mb-2 flex h-9 w-9 items-center justify-center rounded-md border bg-muted/30"><TriangleAlert className="h-4 w-4 text-warning" /></div><DialogTitle>{title}</DialogTitle><DialogDescription>{description}</DialogDescription></DialogHeader><DialogFooter><Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button><Button variant={destructive ? "destructive" : "default"} onClick={() => { onConfirm(); onOpenChange(false); }}>{confirmLabel}</Button></DialogFooter></DialogContent></Dialog>;
}
