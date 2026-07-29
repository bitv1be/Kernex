import { create } from "zustand";
import type { AgentEvent, PendingApproval, SessionRecord, Settings, WorkspaceOverview } from "./types";

interface AppStore {
  workspace: string;
  overview?: WorkspaceOverview;
  session?: SessionRecord;
  running: boolean;
  streamedAnswer: string;
  events: AgentEvent[];
  approval?: PendingApproval;
  selectedFile?: string;
  fileContent: string;
  diff: string;
  error?: string;
  settings?: Settings;
  setWorkspace: (workspace: string, overview: WorkspaceOverview) => void;
  setSession: (session?: SessionRecord) => void;
  setRunning: (running: boolean) => void;
  appendEvent: (event: AgentEvent) => void;
  setApproval: (approval?: PendingApproval) => void;
  setFile: (path?: string, content?: string) => void;
  setDiff: (diff: string) => void;
  setError: (error?: string) => void;
  setSettings: (settings: Settings) => void;
  resetStream: () => void;
}

export const useAppStore = create<AppStore>((set) => ({
  workspace: "",
  running: false,
  streamedAnswer: "",
  events: [],
  fileContent: "",
  diff: "",
  setWorkspace: (workspace, overview) => set({ workspace, overview, selectedFile: undefined, fileContent: "", diff: "", error: undefined }),
  setSession: (session) => set({ session, streamedAnswer: "", events: [], error: undefined }),
  setRunning: (running) => set({ running }),
  appendEvent: (event) => set((state) => ({
    events: [...state.events, event],
    streamedAnswer: event.type === "model_delta" && event.event.type === "text_delta"
      ? state.streamedAnswer + event.event.text
      : state.streamedAnswer,
  })),
  setApproval: (approval) => set({ approval }),
  setFile: (selectedFile, fileContent = "") => set({ selectedFile, fileContent }),
  setDiff: (diff) => set({ diff }),
  setError: (error) => set({ error }),
  setSettings: (settings) => set({ settings }),
  resetStream: () => set({ streamedAnswer: "", events: [], error: undefined }),
}));
