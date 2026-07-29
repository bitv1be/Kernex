import { FileDiff, FileMinus, FilePlus, FileWarning } from "lucide-react";
import { useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/copy-button";
import { parseDiff } from "@/lib/diff";

export function DiffLines({ diff }: { diff: string }) {
  return <pre className="min-w-max font-mono text-[10px] leading-5">{diff.split("\n").map((line, index) => <span key={index} className={`block px-3 ${line.startsWith("+") && !line.startsWith("+++") ? "diff-add" : line.startsWith("-") && !line.startsWith("---") ? "diff-remove" : ""}`}><span className="mr-3 inline-block w-8 select-none text-right text-muted-foreground/40">{index + 1}</span>{line || " "}</span>)}</pre>;
}

export function FileChange({ diff, onOpenFile }: { diff: string; onOpenFile?: (path: string) => void }) {
  const [open, setOpen] = useState(true);
  const parsed = useMemo(() => parseDiff(diff), [diff]);
  const Icon = parsed.status === "created" ? FilePlus : parsed.status === "deleted" ? FileMinus : parsed.status === "conflicted" ? FileWarning : FileDiff;
  return <section className="content-auto overflow-hidden rounded-md border bg-card/40">
    <div className="flex items-center gap-2 px-3 py-2">
      <Icon className="h-3.5 w-3.5 text-muted-foreground" />
      <button className="min-w-0 flex-1 truncate text-left font-mono text-[11px] hover:underline" title={parsed.path} onClick={() => onOpenFile?.(parsed.path)}>{parsed.path}</button>
      <Badge variant="outline">{parsed.status}</Badge>
      <span className="text-[10px] text-success">+{parsed.added}</span>
      <span className="text-[10px] text-destructive">−{parsed.removed}</span>
      <CopyButton value={parsed.path} label="Copy path" className="h-6 px-1.5" />
      <Button size="sm" variant="ghost" className="h-6 px-1.5 text-[10px]" onClick={() => setOpen((value) => !value)}>{open ? "Hide diff" : "View diff"}</Button>
    </div>
    {open && <div className="max-h-[420px] overflow-auto border-t bg-code py-2"><DiffLines diff={diff} /></div>}
  </section>;
}
