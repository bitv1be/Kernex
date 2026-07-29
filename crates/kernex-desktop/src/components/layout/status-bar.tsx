import { Braces, CircleGauge, GitBranch, ShieldCheck } from "lucide-react";
import { StatusIndicator } from "@/components/shared/status-indicator";
import { useAppStore } from "@/lib/store";

export function StatusBar() {
  const running = useAppStore((state) => state.running);
  const session = useAppStore((state) => state.session);
  const settings = useAppStore((state) => state.settings);
  const workspace = useAppStore((state) => state.workspace);
  const overview = useAppStore((state) => state.overview);
  const branch = overview?.gitStatus.split("\n").find((line) => line.startsWith("##"))?.replace(/^##\s*/, "").split("...")[0];
  const input = session?.token_usage.input_tokens ?? 0;
  const output = session?.token_usage.output_tokens ?? 0;
  return <footer className="flex h-6 shrink-0 items-center justify-between gap-4 border-t bg-card px-2 text-[9px] text-muted-foreground">
    <div className="flex min-w-0 items-center gap-3">
      <StatusIndicator status={running ? "running" : "idle"} label={running ? "Agent running" : "Ready"} />
      {overview?.isGitRepository && <span className="flex min-w-0 items-center gap-1"><GitBranch className="h-3 w-3" /><span className="max-w-36 truncate">{branch || "Git repository"}</span></span>}
      {workspace && <span className="hidden items-center gap-1 lg:flex"><Braces className="h-3 w-3" />{overview?.files.length ?? 0} files indexed</span>}
    </div>
    <div className="flex items-center gap-3">
      <span className="hidden items-center gap-1 md:flex"><ShieldCheck className="h-3 w-3" />{settings?.permission_mode ?? "loading"}</span>
      <span className="flex items-center gap-1"><CircleGauge className="h-3 w-3" />{session ? `${input.toLocaleString()} in · ${output.toLocaleString()} out` : "No active context"}</span>
    </div>
  </footer>;
}
