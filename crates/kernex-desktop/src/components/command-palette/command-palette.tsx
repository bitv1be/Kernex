import { Bot, Files, FolderOpen, MessageSquare, PanelLeft, PanelRight, Plus, Search, Settings2, SlidersHorizontal } from "lucide-react";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "@/components/ui/command";
import type { SessionRecord, Settings, WorkspaceOverview } from "@/lib/types";
import { useAppStore } from "@/lib/store";

function sessionTitle(session: SessionRecord) {
  return session.messages.find((message) => message.role === "user")?.content.trim() || "New session";
}

export function AppCommandPalette({ sessions, settings, overview, onNewSession, onOpenProject, onSession, onFile }: { sessions: SessionRecord[]; settings?: Settings; overview?: WorkspaceOverview; onNewSession: () => void; onOpenProject: () => void; onSession: (session: SessionRecord) => void; onFile: (path: string) => void }) {
  const open = useAppStore((state) => state.commandPaletteOpen);
  const setOpen = useAppStore((state) => state.setCommandPaletteOpen);
  const cycleSidebar = useAppStore((state) => state.cycleSidebar);
  const contextOpen = useAppStore((state) => state.contextOpen);
  const setContextOpen = useAppStore((state) => state.setContextOpen);
  const setSettingsOpen = useAppStore((state) => state.setSettingsOpen);
  const run = (action: () => void) => { setOpen(false); action(); };
  return <CommandDialog open={open} onOpenChange={setOpen}>
    <CommandInput placeholder="Search commands, sessions, and files…" />
    <CommandList>
      <CommandEmpty>No matching command.</CommandEmpty>
      <CommandGroup heading="Actions">
        <CommandItem onSelect={() => run(onNewSession)}><Plus />New session<CommandShortcut>Ctrl N</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(onOpenProject)}><FolderOpen />Open project<CommandShortcut>Ctrl O</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => window.dispatchEvent(new Event("kernex:search-messages")))}><Search />Search messages<CommandShortcut>Ctrl F</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => window.dispatchEvent(new Event("kernex:focus-composer")))}><Bot />Focus composer<CommandShortcut>Ctrl L</CommandShortcut></CommandItem>
      </CommandGroup>
      <CommandSeparator />
      <CommandGroup heading="View">
        <CommandItem onSelect={() => run(cycleSidebar)}><PanelLeft />Toggle sidebar<CommandShortcut>Ctrl B</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => setContextOpen(!contextOpen))}><PanelRight />Toggle context panel<CommandShortcut>Ctrl ⇧ B</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => setSettingsOpen(true))}><Settings2 />Open settings<CommandShortcut>Ctrl ,</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => setSettingsOpen(true, "providers"))}><SlidersHorizontal />Change provider<CommandShortcut>{settings?.provider.name}</CommandShortcut></CommandItem>
        <CommandItem onSelect={() => run(() => setSettingsOpen(true, "models"))}><Bot />Change model<CommandShortcut>{settings?.provider.model || "unset"}</CommandShortcut></CommandItem>
      </CommandGroup>
      {sessions.length > 0 && <><CommandSeparator /><CommandGroup heading="Recent sessions">{sessions.slice(0, 8).map((session) => <CommandItem key={session.id} value={`session ${sessionTitle(session)} ${session.provider} ${session.model}`} onSelect={() => run(() => onSession(session))}><MessageSquare /><span className="min-w-0 flex-1 truncate">{sessionTitle(session)}</span><CommandShortcut>{session.status}</CommandShortcut></CommandItem>)}</CommandGroup></>}
      {overview?.files.length ? <><CommandSeparator /><CommandGroup heading="Recent files">{overview.files.slice(0, 12).map((file) => <CommandItem key={file.path} value={`file ${file.path}`} onSelect={() => run(() => onFile(file.path))}><Files /><span className="truncate font-mono text-xs">{file.path}</span></CommandItem>)}</CommandGroup></> : null}
    </CommandList>
  </CommandDialog>;
}
