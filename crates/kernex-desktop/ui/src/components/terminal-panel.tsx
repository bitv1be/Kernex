import { useEffect, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { api } from "@/lib/api";

function splitCommand(input: string): string[] {
  return (input.match(/(?:[^\s"']+|"[^"]*"|'[^']*')+/g) ?? []).map((part) => part.replace(/^("|')|("|')$/g, ""));
}

export function TerminalPanel({ workspace }: { workspace: string }) {
  const host = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef(workspace);

  useEffect(() => {
    workspaceRef.current = workspace;
  }, [workspace]);

  useEffect(() => {
    if (!host.current) return;
    const terminal = new Terminal({ convertEol: true, cursorBlink: true, fontSize: 12, theme: { background: "#09090b", foreground: "#e4e4e7", green: "#34d399" } });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host.current);
    fit.fit();
    terminal.writeln("Kernex permissioned terminal");
    terminal.write("\r\n$ ");
    let line = "";
    const input = terminal.onData(async (data) => {
      if (data === "\r") {
        terminal.write("\r\n");
        const command = splitCommand(line);
        line = "";
        if (command.length > 0 && workspaceRef.current) {
          try {
            const output = await api.runTerminal(workspaceRef.current, command);
            terminal.write(output.stdout.replaceAll("\n", "\r\n"));
            terminal.write(output.stderr.replaceAll("\n", "\r\n"));
            if (!output.success) terminal.writeln(`\r\n[exit ${output.exit_code ?? "unknown"}]`);
          } catch (error) {
            terminal.writeln(`\r\n[error] ${String(error)}`);
          }
        }
        terminal.write("\r\n$ ");
      } else if (data === "\u007f") {
        if (line.length > 0) { line = line.slice(0, -1); terminal.write("\b \b"); }
      } else if (data >= " ") {
        line += data;
        terminal.write(data);
      }
    });
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(host.current);
    return () => { observer.disconnect(); input.dispose(); terminal.dispose(); };
  }, []);

  return <div ref={host} className="h-full min-h-0 bg-[#09090b] p-2" />;
}
