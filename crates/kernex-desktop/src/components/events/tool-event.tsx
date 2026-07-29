import { CheckCircle2, ChevronDown, CircleDashed, FileCode2, TerminalSquare, TriangleAlert, Wrench } from "lucide-react";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/shared/copy-button";
import type { ToolCall, ToolResult } from "@/lib/types";

type ToolState = "running" | "completed" | "failed";

function stringify(value: unknown) {
  if (typeof value === "string") return value;
  try { return JSON.stringify(value, null, 2); } catch { return String(value); }
}

function isTerminal(name: string) {
  return /(shell|command|terminal|exec|run)/i.test(name);
}

function isFile(name: string) {
  return /(file|read|write|edit|patch|diff)/i.test(name);
}

export function ToolEvent({ call, result, state = result?.error ? "failed" : result ? "completed" : "running", output, error, timestamp }: { call?: ToolCall; result?: ToolResult; state?: ToolState; output?: string; error?: string; timestamp?: string }) {
  const [open, setOpen] = useState(state === "failed");
  const name = call?.name ?? result?.name ?? "Tool";
  const details = call ? stringify(call.arguments) : "";
  const value = error ?? result?.error ?? output ?? result?.result ?? "";
  const Icon = isTerminal(name) ? TerminalSquare : isFile(name) ? FileCode2 : Wrench;
  const StateIcon = state === "running" ? CircleDashed : state === "failed" ? TriangleAlert : CheckCircle2;
  const variant = state === "failed" ? "destructive" : state === "completed" ? "success" : "outline";
  return <section className="content-auto overflow-hidden rounded-md border bg-card/45" aria-label={`${name} tool ${state}`}>
    <button className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-muted/35" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
      <Icon className="h-3.5 w-3.5 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate text-xs font-medium">{name.replaceAll("_", " ")}</span>
      {timestamp && <time className="hidden text-[9px] text-muted-foreground sm:block">{new Date(timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>}
      <Badge variant={variant}><StateIcon className={state === "running" ? "animate-spin" : ""} />{state}</Badge>
      <ChevronDown className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${open ? "rotate-180" : ""}`} />
    </button>
    {open && <div className="border-t">
      {details && <div className="border-b p-3"><div className="mb-1.5 text-[9px] font-medium uppercase tracking-wider text-muted-foreground">Input</div><pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all font-mono text-[10px] leading-5">{details}</pre></div>}
      {value && <div className={isTerminal(name) ? "bg-terminal text-zinc-200" : "bg-code"}>
        <div className="flex h-7 items-center justify-between border-b border-white/10 px-2 text-[9px] uppercase tracking-wider text-zinc-400"><span>{error || result?.error ? "Error" : "Output"}</span><CopyButton value={value} label="Copy output" className="h-6 text-zinc-400 hover:bg-white/10 hover:text-white" /></div>
        <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-[10px] leading-5">{value}</pre>
      </div>}
      {!details && !value && <p className="p-3 text-xs text-muted-foreground">Waiting for tool details…</p>}
    </div>}
  </section>;
}
