import CodeMirror from "@uiw/react-codemirror";
import { Activity, Code2, GitCompare, TerminalSquare } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAppStore } from "@/lib/store";
import { TerminalPanel } from "./terminal-panel";

function DiffView({ diff }: { diff: string }) {
  return <pre className="min-h-full whitespace-pre-wrap p-3 text-[11px] leading-5">{diff.split("\n").map((line, index) => <div key={index} className={line.startsWith("+") && !line.startsWith("+++") ? "diff-add" : line.startsWith("-") && !line.startsWith("---") ? "diff-remove" : ""}>{line || " "}</div>)}</pre>;
}

export function InspectorPanel() {
  const workspace = useAppStore((state) => state.workspace);
  const file = useAppStore((state) => state.selectedFile);
  const content = useAppStore((state) => state.fileContent);
  const diff = useAppStore((state) => state.diff);
  const events = useAppStore((state) => state.events);
  return (
    <aside className="hidden w-[38%] min-w-[390px] max-w-[620px] shrink-0 border-l bg-card/30 xl:flex xl:flex-col">
      <Tabs defaultValue="code" className="flex min-h-0 flex-1 flex-col">
        <div className="border-b p-2"><TabsList className="w-full justify-start"><TabsTrigger value="code"><Code2 className="mr-1 h-3.5 w-3.5" />Code</TabsTrigger><TabsTrigger value="diff"><GitCompare className="mr-1 h-3.5 w-3.5" />Diff</TabsTrigger><TabsTrigger value="terminal"><TerminalSquare className="mr-1 h-3.5 w-3.5" />Terminal</TabsTrigger><TabsTrigger value="activity"><Activity className="mr-1 h-3.5 w-3.5" />Activity</TabsTrigger></TabsList></div>
        <TabsContent value="code" className="min-h-0 flex-1 p-0"><div className="border-b px-3 py-2 text-xs text-muted-foreground">{file || "Select a file"}</div><ScrollArea className="h-[calc(100%-34px)]"><CodeMirror value={content} readOnly theme="dark" basicSetup={{ lineNumbers: true, foldGutter: true }} /></ScrollArea></TabsContent>
        <TabsContent value="diff" className="min-h-0 flex-1 p-0"><ScrollArea className="h-full"><DiffView diff={diff || "No working-tree diff."} /></ScrollArea></TabsContent>
        <TabsContent value="terminal" className="min-h-0 flex-1 p-0"><TerminalPanel workspace={workspace} /></TabsContent>
        <TabsContent value="activity" className="min-h-0 flex-1 p-0"><ScrollArea className="h-full"><div className="space-y-2 p-3">{events.map((event, index) => <div key={index} className="rounded border bg-background/60 p-2 text-xs"><span className="font-medium text-emerald-400">{event.type.replaceAll("_", " ")}</span><pre className="mt-1 overflow-auto whitespace-pre-wrap text-[10px] text-muted-foreground">{JSON.stringify(event, null, 2)}</pre></div>)}</div></ScrollArea></TabsContent>
      </Tabs>
    </aside>
  );
}
