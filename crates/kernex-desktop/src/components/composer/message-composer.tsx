import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Bot, CircleStop, FilePlus2, FolderOpen, Gauge, Paperclip, Send, Shield, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { KeyboardKey } from "@/components/shared/keyboard-key";
import { useAppStore, type AgentMode } from "@/lib/store";
import type { PermissionMode, ProviderKind, Settings } from "@/lib/types";

const providerKinds: ProviderKind[] = ["codex", "openai-compatible", "anthropic", "gemini", "local", "custom"];

export interface ComposerSubmission {
  task: string;
  attachments: string[];
  mode: AgentMode;
}

function workspaceRelative(workspace: string, path: string) {
  const root = workspace.replace(/[\\/]+$/, "");
  if (path === root) return ".";
  if (!path.startsWith(`${root}/`) && !path.startsWith(`${root}\\`)) return undefined;
  return path.slice(root.length + 1).replaceAll("\\", "/");
}

export function MessageComposer({ onRun, onCancel, onUpdateSettings }: { onRun: (submission: ComposerSubmission) => Promise<void>; onCancel: () => Promise<void>; onUpdateSettings: (settings: Settings) => Promise<void> }) {
  const [task, setTask] = useState("");
  const [attachments, setAttachments] = useState<string[]>([]);
  const [validation, setValidation] = useState<string>();
  const [dragging, setDragging] = useState(false);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const workspace = useAppStore((state) => state.workspace);
  const running = useAppStore((state) => state.running);
  const settings = useAppStore((state) => state.settings);
  const session = useAppStore((state) => state.session);
  const mode = useAppStore((state) => state.agentMode);
  const setMode = useAppStore((state) => state.setAgentMode);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const usage = (session?.token_usage.input_tokens ?? 0) + (session?.token_usage.output_tokens ?? 0);
  const addPaths = useCallback((paths: string[]) => {
    if (!workspace) { setValidation("Open a project before attaching files."); return; }
    const accepted = paths.filter((path) => workspaceRelative(workspace, path));
    if (accepted.length !== paths.length) setValidation("Attachments must be inside the current workspace so the agent can access them safely.");
    else setValidation(undefined);
    setAttachments((current) => [...new Set([...current, ...accepted])].slice(0, 20));
  }, [workspace]);

  useEffect(() => {
    const focus = (event: Event) => {
      const detail = (event as CustomEvent<string>).detail;
      if (detail) setTask(detail);
      requestAnimationFrame(() => textarea.current?.focus());
    };
    window.addEventListener("kernex:focus-composer", focus);
    window.addEventListener("kernex:set-composer", focus);
    return () => { window.removeEventListener("kernex:focus-composer", focus); window.removeEventListener("kernex:set-composer", focus); };
  }, []);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "over") setDragging(true);
      if (event.payload.type === "leave") setDragging(false);
      if (event.payload.type === "drop") {
        setDragging(false);
        addPaths(event.payload.paths);
      }
    }).then((unlisten) => { dispose = unlisten; }).catch(() => undefined);
    return () => dispose?.();
  }, [addPaths]);

  useEffect(() => {
    if (!textarea.current) return;
    textarea.current.style.height = "0px";
    textarea.current.style.height = `${Math.min(176, Math.max(48, textarea.current.scrollHeight))}px`;
  }, [task]);

  const relativeAttachments = useMemo(() => attachments.map((path) => workspaceRelative(workspace, path)).filter((path): path is string => Boolean(path)), [attachments, workspace]);
  const chooseFiles = async () => {
    const selected = await open({ title: "Attach workspace files", multiple: true, directory: false, defaultPath: workspace || undefined });
    if (!selected) return;
    addPaths(Array.isArray(selected) ? selected : [selected]);
  };
  const submit = async () => {
    const value = task.trim();
    if (!workspace) { setValidation("Open a project before starting a task."); return; }
    if (!settings?.provider.model.trim()) { setValidation("Choose a model before starting a task."); setSettingsOpen(true, "models"); return; }
    if (!value || running) return;
    setValidation(undefined);
    setTask("");
    setAttachments([]);
    await onRun({ task: value, attachments: relativeAttachments, mode });
  };
  const patchSettings = async (patch: Partial<Settings>) => {
    if (!settings) return;
    await onUpdateSettings({ ...settings, ...patch });
  };

  return <div className="shrink-0 bg-gradient-to-t from-background via-background to-transparent px-3 pb-3 pt-2">
    <div className={`relative mx-auto max-w-4xl overflow-hidden rounded-lg border bg-card shadow-sm transition-colors focus-within:border-ring ${dragging ? "border-foreground bg-muted/50" : ""}`}>
      {dragging && <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center bg-background/90 text-xs font-medium"><FilePlus2 className="mr-2 h-4 w-4" />Drop workspace files to attach</div>}
      {attachments.length > 0 && <div className="flex flex-wrap gap-1.5 border-b px-3 py-2">{relativeAttachments.map((path, index) => <Badge key={path} variant="secondary" className="max-w-64 normal-case"><Paperclip className="h-3 w-3" /><span className="truncate">{path}</span><button onClick={() => setAttachments((items) => items.filter((_, itemIndex) => itemIndex !== index))} aria-label={`Remove ${path}`}><X className="h-3 w-3" /></button></Badge>)}</div>}
      <Textarea ref={textarea} data-composer value={task} onChange={(event) => setTask(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} placeholder={workspace ? "Describe a task, ask a question, or request a change…" : "Open a project to begin…"} className="min-h-12 max-h-44 resize-none overflow-y-auto rounded-none border-0 bg-transparent px-3 py-3 text-[13px] leading-5 shadow-none focus-visible:ring-0" disabled={running || !workspace} aria-describedby={validation ? "composer-validation" : undefined} />
      {validation && <p id="composer-validation" role="alert" className="border-t border-destructive/20 bg-destructive/5 px-3 py-1.5 text-[10px] text-destructive">{validation}</p>}
      <div className="flex min-h-9 flex-wrap items-center gap-1 border-t px-2 py-1">
        <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => void chooseFiles()} disabled={!workspace || running} aria-label="Attach files"><Paperclip className="h-3.5 w-3.5" /></Button></TooltipTrigger><TooltipContent>Attach workspace files</TooltipContent></Tooltip>
        <Select value={mode} onValueChange={(value: AgentMode) => setMode(value)} disabled={running}><SelectTrigger className="h-7 w-auto min-w-24 gap-1 border-0 px-2 text-[10px] shadow-none"><Bot className="h-3 w-3" /><SelectValue /></SelectTrigger><SelectContent><SelectItem value="agent">Agent</SelectItem><SelectItem value="plan">Plan only</SelectItem><SelectItem value="review">Review</SelectItem></SelectContent></Select>
        {settings && <Select value={settings.provider.name} onValueChange={(name: ProviderKind) => void patchSettings({ provider: { ...settings.provider, name } })} disabled={running}><SelectTrigger className="h-7 w-auto max-w-40 gap-1 border-0 px-2 text-[10px] shadow-none"><SelectValue /></SelectTrigger><SelectContent>{providerKinds.map((provider) => <SelectItem key={provider} value={provider}>{provider}</SelectItem>)}</SelectContent></Select>}
        <Button variant="ghost" size="sm" className="h-7 max-w-44 px-2 text-[10px] font-normal" onClick={() => setSettingsOpen(true, "models")} disabled={running}><span className="truncate">{settings?.provider.model || "Choose model"}</span></Button>
        {settings && <Select value={settings.permission_mode} onValueChange={(permission_mode: PermissionMode) => void patchSettings({ permission_mode })} disabled={running}><SelectTrigger className="h-7 w-auto gap-1 border-0 px-2 text-[10px] shadow-none"><Shield className="h-3 w-3" /><SelectValue /></SelectTrigger><SelectContent><SelectItem value="read-only">Read only</SelectItem><SelectItem value="ask">Ask</SelectItem><SelectItem value="auto-safe">Auto safe</SelectItem><SelectItem value="full-access">Full access</SelectItem></SelectContent></Select>}
        <div className="ml-auto flex items-center gap-2 text-[9px] text-muted-foreground">
          <Tooltip><TooltipTrigger asChild><span className="hidden max-w-40 items-center gap-1 truncate lg:flex"><FolderOpen className="h-3 w-3 shrink-0" />{workspace.split(/[\\/]/).filter(Boolean).at(-1) ?? "No workspace"}</span></TooltipTrigger><TooltipContent>{workspace}</TooltipContent></Tooltip>
          <span className="hidden items-center gap-1 sm:flex"><Gauge className="h-3 w-3" />{usage.toLocaleString()} tokens</span>
          {!running && <span className="hidden items-center gap-1 xl:flex"><KeyboardKey>Enter</KeyboardKey> send</span>}
          {running ? <Button size="sm" variant="outline" className="h-7 border-destructive/40 px-2 text-[10px] text-destructive" onClick={() => void onCancel()}><CircleStop className="h-3.5 w-3.5" />Stop</Button> : <Button size="icon" className="h-7 w-7" onClick={() => void submit()} disabled={!task.trim() || !workspace} aria-label="Send message"><Send className="h-3.5 w-3.5" /></Button>}
        </div>
      </div>
    </div>
  </div>;
}
