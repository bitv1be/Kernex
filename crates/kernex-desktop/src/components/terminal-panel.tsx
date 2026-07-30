import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { Clipboard, Play, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import "@xterm/xterm/css/xterm.css";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";

function splitCommand(input: string): string[] {
  return (input.match(/(?:[^\s"']+|"[^"]*"|'[^']*')+/g) ?? []).map((part) => part.replace(/^("|')|("|')$/g, ""));
}

export function TerminalPanel({ workspace }: { workspace: string }) {
  const host = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const workspaceRef = useRef(workspace);
  const runRef = useRef<(command: string[]) => Promise<void>>(async () => undefined);
  const lastCommandRef = useRef<string[]>([]);
  const busyRef = useRef(false);
  const [lastCommand, setLastCommand] = useState<string[]>([]);
  const [lastOutput, setLastOutput] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => { workspaceRef.current = workspace; }, [workspace]);
  useEffect(() => {
    if (!host.current) return;
    const terminal = new Terminal({ convertEol: true, cursorBlink: true, fontSize: 12, lineHeight: 1.35, scrollback: 10_000, theme: { background: "#0c0c0e", foreground: "#dedee2", cursor: "#dedee2", selectionBackground: "#3a3a40", black: "#0c0c0e", red: "#c87979", green: "#9ab2a1", yellow: "#c0a574", blue: "#9babc4", magenta: "#b2a0bd", cyan: "#93b4b4", white: "#dedee2" } });
    terminalRef.current = terminal;
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host.current);
    fit.fit();
    terminal.writeln("Kernex permissioned terminal");
    terminal.writeln("Commands run in the active workspace and may request approval.\r\n");
    terminal.write("$ ");
    let line = "";
    const runCommand = async (command: string[]) => {
      if (!command.length || !workspaceRef.current) return;
      busyRef.current = true; lastCommandRef.current = command;
      setBusy(true); setLastCommand(command);
      const started = performance.now();
      try {
        const output = await api.runTerminal(workspaceRef.current, command);
        const text = `${output.stdout}${output.stderr}`;
        setLastOutput(text);
        terminal.write(output.stdout.replaceAll("\n", "\r\n"));
        terminal.write(output.stderr.replaceAll("\n", "\r\n"));
        const duration = Math.round(performance.now() - started);
        terminal.writeln(`\r\n[${output.success ? "completed" : `exit ${output.exit_code ?? "unknown"}`} · ${duration} ms${output.truncated ? " · truncated" : ""}]`);
      } catch (error) {
        const message = String(error);
        setLastOutput(message);
        terminal.writeln(`\r\n[error] ${message}`);
      } finally {
        busyRef.current = false;
        setBusy(false);
        terminal.write("\r\n$ ");
      }
    };
    runRef.current = runCommand;
    const input = terminal.onData((data) => {
      if (data === "\r") {
        terminal.write("\r\n");
        const command = splitCommand(line);
        line = "";
        void runCommand(command);
      } else if (data === "\u007f") {
        if (line.length > 0) { line = line.slice(0, -1); terminal.write("\b \b"); }
      } else if (data === "\u001b[A" && lastCommandRef.current.length) {
        while (line.length) { terminal.write("\b \b"); line = line.slice(0, -1); }
        line = lastCommandRef.current.join(" "); terminal.write(line);
      } else if (data >= " " && !busyRef.current) {
        line += data;
        terminal.write(data);
      }
    });
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(host.current);
    return () => { observer.disconnect(); input.dispose(); terminal.dispose(); terminalRef.current = null; };
  }, []);

  return <div className="flex h-full min-h-0 flex-col bg-terminal">
    <div className="flex h-8 shrink-0 items-center gap-1 border-b border-white/10 px-2 text-[9px] text-zinc-400"><span className="mr-auto truncate font-mono">{workspace || "No workspace"}</span><Button variant="ghost" size="icon" className="h-6 w-6 text-zinc-400 hover:bg-white/10 hover:text-white" onClick={() => void navigator.clipboard.writeText(lastOutput)} disabled={!lastOutput} aria-label="Copy last output"><Clipboard className="h-3 w-3" /></Button><Button variant="ghost" size="icon" className="h-6 w-6 text-zinc-400 hover:bg-white/10 hover:text-white" onClick={() => void runRef.current(lastCommand)} disabled={!lastCommand.length || busy} aria-label="Rerun last command"><Play className="h-3 w-3" /></Button><Button variant="ghost" size="icon" className="h-6 w-6 text-zinc-400 hover:bg-white/10 hover:text-white" onClick={() => { terminalRef.current?.reset(); terminalRef.current?.write("$ "); }} aria-label="Reset terminal"><RotateCcw className="h-3 w-3" /></Button><Button variant="ghost" size="icon" className="h-6 w-6 text-zinc-400 hover:bg-white/10 hover:text-white" onClick={() => terminalRef.current?.clear()} aria-label="Clear terminal"><Trash2 className="h-3 w-3" /></Button></div>
    <div ref={host} className="min-h-0 flex-1 p-2" />
  </div>;
}
