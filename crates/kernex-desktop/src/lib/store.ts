import { create } from "zustand";
import { createJSONStorage, persist, type StateStorage } from "zustand/middleware";
import type { AgentEvent, PendingApproval, SessionRecord, Settings, WorkspaceOverview } from "./types";

export type SidebarMode = "expanded" | "compact" | "hidden";
export type ContextTab = "files" | "changes" | "terminal" | "activity" | "context";
export type AgentMode = "agent" | "plan" | "review";

const uiStorage: StateStorage = {
  getItem: (name) => { try { return window.localStorage.getItem(name); } catch { return null; } },
  setItem: (name, value) => { try { window.localStorage.setItem(name, value); } catch { /* Persistence is best-effort in restricted webviews and tests. */ } },
  removeItem: (name) => { try { window.localStorage.removeItem(name); } catch { /* Persistence is best-effort in restricted webviews and tests. */ } },
};

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
  sidebarMode: SidebarMode;
  sidebarWidth: number;
  contextOpen: boolean;
  contextWidth: number;
  contextTab: ContextTab;
  settingsOpen: boolean;
  settingsSection: string;
  commandPaletteOpen: boolean;
  agentMode: AgentMode;
  favoriteModels: string[];
  notifyOnComplete: boolean;
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
  setSidebarMode: (mode: SidebarMode) => void;
  cycleSidebar: () => void;
  setSidebarWidth: (width: number) => void;
  setContextOpen: (open: boolean) => void;
  setContextWidth: (width: number) => void;
  setContextTab: (tab: ContextTab) => void;
  setSettingsOpen: (open: boolean, section?: string) => void;
  setCommandPaletteOpen: (open: boolean) => void;
  setAgentMode: (mode: AgentMode) => void;
  toggleFavoriteModel: (model: string) => void;
  setNotifyOnComplete: (enabled: boolean) => void;
}

export const useAppStore = create<AppStore>()(persist((set) => ({
  workspace: "",
  running: false,
  streamedAnswer: "",
  events: [],
  fileContent: "",
  diff: "",
  sidebarMode: "expanded",
  sidebarWidth: 272,
  contextOpen: true,
  contextWidth: 420,
  contextTab: "files",
  settingsOpen: false,
  settingsSection: "appearance",
  commandPaletteOpen: false,
  agentMode: "agent",
  favoriteModels: [],
  notifyOnComplete: true,
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
  setSidebarMode: (sidebarMode) => set({ sidebarMode }),
  cycleSidebar: () => set((state) => ({ sidebarMode: state.sidebarMode === "expanded" ? "compact" : state.sidebarMode === "compact" ? "hidden" : "expanded" })),
  setSidebarWidth: (sidebarWidth) => set({ sidebarWidth: Math.min(380, Math.max(232, sidebarWidth)) }),
  setContextOpen: (contextOpen) => set({ contextOpen }),
  setContextWidth: (contextWidth) => set({ contextWidth: Math.min(680, Math.max(340, contextWidth)) }),
  setContextTab: (contextTab) => set({ contextTab, contextOpen: true }),
  setSettingsOpen: (settingsOpen, settingsSection) => set((state) => ({ settingsOpen, settingsSection: settingsSection ?? state.settingsSection })),
  setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),
  setAgentMode: (agentMode) => set({ agentMode }),
  toggleFavoriteModel: (model) => set((state) => ({ favoriteModels: state.favoriteModels.includes(model) ? state.favoriteModels.filter((item) => item !== model) : [...state.favoriteModels, model] })),
  setNotifyOnComplete: (notifyOnComplete) => set({ notifyOnComplete }),
}), {
  name: "kernex-desktop-ui",
  storage: createJSONStorage(() => uiStorage),
  partialize: (state) => ({
    sidebarMode: state.sidebarMode,
    sidebarWidth: state.sidebarWidth,
    contextOpen: state.contextOpen,
    contextWidth: state.contextWidth,
    contextTab: state.contextTab,
    agentMode: state.agentMode,
    favoriteModels: state.favoriteModels,
    notifyOnComplete: state.notifyOnComplete,
  }),
}));
