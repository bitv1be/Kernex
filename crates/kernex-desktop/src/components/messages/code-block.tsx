import { ChevronDown, ChevronUp } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/shared/copy-button";

export function CodeBlock({ code, language = "text", fileName, highlighted }: { code: string; language?: string; fileName?: string; highlighted?: ReactNode }) {
  const normalized = code.replace(/\n$/, "");
  const lines = useMemo(() => normalized.split("\n"), [normalized]);
  const long = lines.length > 18 || normalized.length > 2400;
  const [expanded, setExpanded] = useState(!long);
  return <figure className="my-3 overflow-hidden rounded-md border bg-code">
    <figcaption className="flex h-8 items-center justify-between border-b px-2 text-[10px] text-muted-foreground">
      <div className="flex min-w-0 items-center gap-2"><span className="font-mono uppercase tracking-wide">{language}</span>{fileName && <><span>·</span><span className="truncate font-mono">{fileName}</span></>}</div>
      <div className="flex items-center"><CopyButton value={normalized} label="Copy code" className="h-6 px-1.5" />{long && <Button variant="ghost" size="sm" className="h-6 px-1.5" onClick={() => setExpanded((value) => !value)} aria-expanded={expanded}>{expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}<span className="text-[10px]">{expanded ? "Collapse" : `Show ${lines.length} lines`}</span></Button>}</div>
    </figcaption>
    <div className={`overflow-auto ${!expanded ? "max-h-64" : "max-h-[680px]"}`}>
      <pre className="min-w-max p-3 font-mono text-[11px] leading-5"><code>{highlighted ?? lines.map((line, index) => <span key={index} className="block"><span aria-hidden="true" className="mr-4 inline-block w-7 select-none text-right text-muted-foreground/45">{index + 1}</span><span>{line || " "}</span></span>)}</code></pre>
    </div>
  </figure>;
}
