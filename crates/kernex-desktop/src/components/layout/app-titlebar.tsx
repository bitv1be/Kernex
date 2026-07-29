import { getCurrentWindow } from "@tauri-apps/api/window";
import { ChevronLeft, ChevronRight, Command, Maximize2, Minus, PanelLeft, PanelRight, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { KernexMark } from "@/components/shared/kernex-mark";
import { useAppStore } from "@/lib/store";

function workspaceName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? "No workspace";
}

export function AppTitlebar({ workspace, canGoBack = false, canGoForward = false }: { workspace: string; canGoBack?: boolean; canGoForward?: boolean }) {
  const sidebarMode = useAppStore((state) => state.sidebarMode);
  const cycleSidebar = useAppStore((state) => state.cycleSidebar);
  const contextOpen = useAppStore((state) => state.contextOpen);
  const setContextOpen = useAppStore((state) => state.setContextOpen);
  const setCommandPaletteOpen = useAppStore((state) => state.setCommandPaletteOpen);
  const window = getCurrentWindow();

  return <header className="relative flex h-9 shrink-0 select-none items-center border-b bg-card text-xs" data-tauri-drag-region onDoubleClick={(event) => { if (!(event.target as HTMLElement).closest("button")) void window.toggleMaximize(); }}>
    <div className="flex min-w-0 items-center gap-1 px-2" data-tauri-drag-region>
      <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" className="h-7 w-7" onClick={cycleSidebar} aria-label="Cycle sidebar"><PanelLeft className="h-3.5 w-3.5" /></Button></TooltipTrigger><TooltipContent>Sidebar · Ctrl B</TooltipContent></Tooltip>
      <Button variant="ghost" size="icon" className="h-7 w-7" disabled={!canGoBack} aria-label="Go back"><ChevronLeft className="h-3.5 w-3.5" /></Button>
      <Button variant="ghost" size="icon" className="h-7 w-7" disabled={!canGoForward} aria-label="Go forward"><ChevronRight className="h-3.5 w-3.5" /></Button>
    </div>
    <div className="pointer-events-none absolute inset-0 flex items-center justify-center gap-2" data-tauri-drag-region>
      <KernexMark className="h-5 w-5 rounded-sm" />
      <span className="font-medium">Kernex</span>
      <span className="text-muted-foreground">/</span>
      <span className="max-w-[34vw] truncate text-muted-foreground">{workspaceName(workspace)}</span>
    </div>
    <div className="ml-auto flex h-full items-center" data-tauri-drag-region>
      <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="sm" className="mr-1 h-7 gap-2 px-2 text-[10px] text-muted-foreground" onClick={() => setCommandPaletteOpen(true)}><Command className="h-3.5 w-3.5" /><span className="hidden lg:inline">Command</span><kbd className="rounded border px-1 text-[9px]">Ctrl K</kbd></Button></TooltipTrigger><TooltipContent>Open command palette</TooltipContent></Tooltip>
      <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon" className="mr-1 h-7 w-7" onClick={() => setContextOpen(!contextOpen)} aria-label="Toggle context panel"><PanelRight className="h-3.5 w-3.5" /></Button></TooltipTrigger><TooltipContent>Context panel · Ctrl Shift B</TooltipContent></Tooltip>
      <div className="h-4 w-px bg-border" />
      <Button variant="ghost" size="icon" className="h-9 w-11 rounded-none" onClick={() => void window.minimize()} aria-label="Minimize window"><Minus className="h-3.5 w-3.5" /></Button>
      <Button variant="ghost" size="icon" className="h-9 w-11 rounded-none" onClick={() => void window.toggleMaximize()} aria-label="Maximize or restore window"><Maximize2 className="h-3 w-3" /></Button>
      <Button variant="ghost" size="icon" className="h-9 w-11 rounded-none hover:bg-destructive hover:text-destructive-foreground" onClick={() => void window.close()} aria-label="Close window"><X className="h-3.5 w-3.5" /></Button>
    </div>
    {sidebarMode === "hidden" && <span className="sr-only">Sidebar hidden</span>}
  </header>;
}
