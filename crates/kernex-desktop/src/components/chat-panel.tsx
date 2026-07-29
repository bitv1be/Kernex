import { Bot, CircleStop, Send, User, Wrench } from "lucide-react";
import { useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { useAppStore } from "@/lib/store";

export function ChatPanel({ onRun, onCancel }: { onRun: (task: string) => Promise<void>; onCancel: () => Promise<void> }) {
  const [task, setTask] = useState("");
  const session = useAppStore((state) => state.session);
  const running = useAppStore((state) => state.running);
  const streamed = useAppStore((state) => state.streamedAnswer);
  const events = useAppStore((state) => state.events);
  const visibleMessages = useMemo(() => session?.messages.filter((message) => message.role === "user" || message.role === "assistant") ?? [], [session]);
  const tools = events.filter((event) => event.type === "tool_started" || event.type === "tool_finished" || event.type === "tool_failed");
  const submit = async () => {
    const value = task.trim();
    if (!value || running) return;
    setTask("");
    await onRun(value);
  };

  return (
    <section className="flex min-w-0 flex-1 flex-col bg-background">
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-6 py-8">
          {visibleMessages.length === 0 && !streamed && (
            <div className="mt-20 text-center">
              <div className="mx-auto mb-5 flex h-14 w-14 items-center justify-center rounded-2xl border bg-card shadow"><Bot className="h-7 w-7 text-emerald-400" /></div>
              <h2 className="text-xl font-semibold">What should we build?</h2>
              <p className="mt-2 text-sm text-muted-foreground">Kernex will inspect this project, show every tool action, and ask before risky changes.</p>
            </div>
          )}
          {visibleMessages.map((message, index) => (
            <article key={`${message.role}-${index}`} className={message.role === "user" ? "ml-auto max-w-[85%] rounded-xl bg-secondary px-4 py-3" : "flex gap-3"}>
              {message.role === "assistant" && <div className="mt-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-emerald-500/15 text-emerald-400"><Bot className="h-4 w-4" /></div>}
              {message.role === "user" && <User className="mr-2 inline h-4 w-4 text-muted-foreground" />}
              {message.role === "assistant" ? <div className="markdown min-w-0"><ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]}>{message.content}</ReactMarkdown></div> : <span className="text-sm">{message.content}</span>}
            </article>
          ))}
          {streamed && (
            <article className="flex gap-3">
              <div className="mt-1 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-emerald-500/15 text-emerald-400"><Bot className="h-4 w-4" /></div>
              <div className="markdown min-w-0"><ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]}>{streamed}</ReactMarkdown><span className="inline-block h-4 w-1 animate-pulse bg-emerald-400" /></div>
            </article>
          )}
          {tools.length > 0 && (
            <div className="space-y-2 border-l border-border pl-4">
              {tools.slice(-8).map((event, index) => (
                <div key={`${event.type}-${index}`} className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Wrench className="h-3.5 w-3.5" />
                  {event.type === "tool_started" && <span>Running <strong className="text-foreground">{event.call.name}</strong></span>}
                  {event.type === "tool_finished" && <span><strong className="text-emerald-400">{event.name}</strong> completed</span>}
                  {event.type === "tool_failed" && <span><strong className="text-red-400">{event.name}</strong> failed: {event.error}</span>}
                </div>
              ))}
            </div>
          )}
        </div>
      </ScrollArea>
      <div className="border-t bg-background/95 p-4 backdrop-blur">
        <div className="mx-auto max-w-3xl rounded-xl border bg-card p-2 shadow-lg focus-within:ring-1 focus-within:ring-ring">
          <Textarea value={task} onChange={(event) => setTask(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void submit(); } }} placeholder="Ask Kernex to inspect, implement, test, or review…" className="min-h-[64px] resize-none border-0 shadow-none focus-visible:ring-0" disabled={running} />
          <div className="flex items-center justify-between px-1 pt-1 text-xs text-muted-foreground">
            <span>Enter to send · Shift+Enter for a new line</span>
            {running ? <Button size="sm" variant="destructive" onClick={() => void onCancel()}><CircleStop className="h-3.5 w-3.5" />Cancel</Button> : <Button size="sm" onClick={() => void submit()} disabled={!task.trim()}><Send className="h-3.5 w-3.5" />Send</Button>}
          </div>
        </div>
      </div>
    </section>
  );
}
