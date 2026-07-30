import { useVirtualizer } from "@tanstack/react-virtual";
import { CheckCircle2, CircleDashed, Search, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { FileChange } from "@/components/events/file-change";
import { PermissionAuditItem } from "@/components/events/permission-audit";
import { ToolEvent } from "@/components/events/tool-event";
import { MessageItem } from "@/components/messages/message-item";
import { MarkdownContent } from "@/components/messages/markdown-content";
import type { AgentEvent, Message, PermissionAudit, SessionRecord, ToolCall, ToolResult } from "@/lib/types";

type TimelineItem =
  | { kind: "message"; key: string; message: Message }
  | { kind: "tool"; key: string; call?: ToolCall; result?: ToolResult; state?: "running" | "completed" | "failed"; output?: string; error?: string }
  | { kind: "diff"; key: string; diff: string }
  | { kind: "permission"; key: string; audit: PermissionAudit }
  | { kind: "progress"; key: string; label: string; done?: boolean }
  | { kind: "stream"; key: string; content: string };

function buildItems(session: SessionRecord | undefined, events: AgentEvent[], streamed: string): TimelineItem[] {
  const items: TimelineItem[] = [];
  const results = new Map(session?.tool_results.map((result) => [result.call_id, result]) ?? []);
  const renderedResults = new Set<string>();
  session?.messages.forEach((message, index) => {
    if (message.role === "tool") {
      const result = message.tool_call_id ? results.get(message.tool_call_id) : undefined;
      if (result) renderedResults.add(result.call_id);
      items.push({ kind: "tool", key: `message-tool-${index}`, result: result ?? { call_id: message.tool_call_id ?? `${index}`, name: message.name ?? "tool", result: message.content, timestamp: session.updated_at } });
    } else {
      items.push({ kind: "message", key: `message-${index}`, message });
    }
  });
  session?.tool_results.filter((result) => !renderedResults.has(result.call_id)).forEach((result, index) => items.push({ kind: "tool", key: `stored-tool-${index}`, result }));
  session?.permission_decisions.forEach((audit, index) => items.push({ kind: "permission", key: `permission-${index}`, audit }));
  session?.generated_diffs.forEach((diff, index) => items.push({ kind: "diff", key: `stored-diff-${index}`, diff }));

  const liveTools = new Map<string, Extract<TimelineItem, { kind: "tool" }>>();
  events.forEach((event, index) => {
    if (event.type === "started") items.push({ kind: "progress", key: `event-${index}`, label: `Preparing ${event.provider} / ${event.model}` });
    if (event.type === "model_requested") items.push({ kind: "progress", key: `event-${index}`, label: `Analyzing project · step ${event.step}` });
    if (event.type === "tool_started") {
      const item: Extract<TimelineItem, { kind: "tool" }> = { kind: "tool", key: `live-tool-${event.call.id}`, call: event.call, state: "running" };
      liveTools.set(event.call.id, item);
      items.push(item);
    }
    if (event.type === "tool_finished" || event.type === "tool_failed") {
      const existing = liveTools.get(event.call_id);
      if (existing) {
        existing.state = event.type === "tool_failed" ? "failed" : "completed";
        if (event.type === "tool_failed") existing.error = event.error;
        else existing.output = event.result;
      } else items.push({ kind: "tool", key: `live-result-${event.call_id}`, result: { call_id: event.call_id, name: event.name, result: event.type === "tool_finished" ? event.result : "", error: event.type === "tool_failed" ? event.error : undefined, timestamp: new Date().toISOString() } });
      if (event.type === "tool_finished" && event.diff) items.push({ kind: "diff", key: `live-diff-${event.call_id}`, diff: event.diff });
    }
    if (event.type === "completed") items.push({ kind: "progress", key: `event-${index}`, label: `Task completed in ${event.steps} steps`, done: true });
  });
  if (streamed) items.push({ kind: "stream", key: "stream", content: streamed });
  return items;
}

export function ConversationTimeline({ session, events, streamed, provider, model, onOpenFile }: { session?: SessionRecord; events: AgentEvent[]; streamed: string; provider?: string; model?: string; onOpenFile: (path: string) => void }) {
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  useEffect(() => {
    const open = () => setSearchOpen(true);
    window.addEventListener("kernex:search-messages", open);
    return () => window.removeEventListener("kernex:search-messages", open);
  }, []);
  const allItems = useMemo(() => buildItems(session, events, streamed), [events, session, streamed]);
  const items = useMemo(() => query.trim() ? allItems.filter((item) => item.kind === "message" && item.message.content.toLowerCase().includes(query.toLowerCase())) : allItems, [allItems, query]);
  const virtual = items.length > 60;
  // TanStack Virtual intentionally exposes mutable measurement functions.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({ count: virtual ? items.length : 0, getScrollElement: () => scrollRef.current, estimateSize: () => 180, overscan: 8 });
  useEffect(() => {
    if (!stickToBottom.current || !scrollRef.current) return;
    requestAnimationFrame(() => { if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight; });
  }, [items.length, streamed.length]);

  const renderItem = (item: TimelineItem) => {
    if (item.kind === "message") return <MessageItem message={item.message} provider={provider} model={model} />;
    if (item.kind === "tool") return <ToolEvent call={item.call} result={item.result} state={item.state} output={item.output} error={item.error} />;
    if (item.kind === "diff") return <FileChange diff={item.diff} onOpenFile={onOpenFile} />;
    if (item.kind === "permission") return <PermissionAuditItem audit={item.audit} />;
    if (item.kind === "progress") return <div role="status" className="flex items-center gap-2 border-l pl-3 text-[11px] text-muted-foreground">{item.done ? <CheckCircle2 className="h-3.5 w-3.5 text-success" /> : <CircleDashed className="h-3.5 w-3.5 animate-spin" />}{item.label}</div>;
    return <article aria-label="Streaming assistant response"><header className="mb-2 flex items-center gap-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground"><CircleDashed className="h-3.5 w-3.5 animate-spin" />Kernex is responding</header><MarkdownContent>{item.content}</MarkdownContent><span className="mt-1 inline-block h-3 w-px animate-pulse bg-foreground" /></article>;
  };

  return <div className="relative min-h-0 flex-1">
    {searchOpen && <div className="absolute right-4 top-3 z-20 flex w-72 items-center gap-1 rounded-md border bg-popover p-1 shadow-lg"><Search className="ml-2 h-3.5 w-3.5 text-muted-foreground" /><Input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search this conversation" className="h-7 border-0 text-xs shadow-none focus-visible:ring-0" /><span className="whitespace-nowrap text-[9px] text-muted-foreground">{query ? items.length : ""}</span><Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => { setSearchOpen(false); setQuery(""); }} aria-label="Close conversation search"><X className="h-3.5 w-3.5" /></Button></div>}
    <div ref={scrollRef} className="h-full overflow-y-auto overflow-x-hidden" onScroll={(event) => { const element = event.currentTarget; stickToBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 120; }}>
      {virtual ? <div className="relative mx-auto w-full max-w-4xl px-6" style={{ height: virtualizer.getTotalSize() }}>{virtualizer.getVirtualItems().map((virtualItem) => <div key={items[virtualItem.index].key} ref={virtualizer.measureElement} data-index={virtualItem.index} className="absolute left-6 right-6 py-4" style={{ transform: `translateY(${virtualItem.start}px)` }}>{renderItem(items[virtualItem.index])}</div>)}</div> : <div className="mx-auto flex w-full max-w-4xl flex-col gap-7 px-6 py-7">{items.map((item) => <div key={item.key}>{renderItem(item)}</div>)}</div>}
    </div>
  </div>;
}
