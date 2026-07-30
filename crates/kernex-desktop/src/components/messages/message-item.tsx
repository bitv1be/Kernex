import { Bot, FileText, UserRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { Message } from "@/lib/types";
import { MarkdownContent } from "./markdown-content";

function splitAttachments(content: string) {
  const match = content.match(/^\[Kernex attachments\]\n([\s\S]*?)\n\[\/Kernex attachments\]\n\n([\s\S]*)$/);
  if (!match) return { content, attachments: [] as string[] };
  return { content: match[2], attachments: match[1].split("\n").map((line) => line.replace(/^-\s*/, "").trim()).filter(Boolean) };
}

export function MessageItem({ message, provider, model }: { message: Message; provider?: string; model?: string }) {
  const parsed = splitAttachments(message.content);
  if (message.role === "system") return <div className="content-auto border-y bg-muted/20 px-4 py-2 text-xs text-muted-foreground">{message.content}</div>;
  if (message.role === "user") return <article className="content-auto border-l-2 border-foreground/55 pl-4" aria-label="User message">
    <header className="mb-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"><UserRound className="h-3.5 w-3.5" />You</header>
    <div className="whitespace-pre-wrap text-[13px] leading-6">{parsed.content}</div>
    {parsed.attachments.length > 0 && <div className="mt-3 flex flex-wrap gap-1.5">{parsed.attachments.map((path) => <Badge key={path} variant="secondary" className="max-w-full normal-case"><FileText className="h-3 w-3" /><span className="truncate">{path}</span></Badge>)}</div>}
  </article>;
  return <article className="content-auto" aria-label="Assistant message">
    <header className="mb-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"><Bot className="h-3.5 w-3.5" />Kernex{provider && <span className="normal-case tracking-normal">· {provider}{model ? ` / ${model}` : ""}</span>}</header>
    <MarkdownContent>{message.content}</MarkdownContent>
  </article>;
}
