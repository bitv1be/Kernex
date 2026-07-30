import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./store";

describe("desktop event store", () => {
  beforeEach(() => useAppStore.setState({ streamedAnswer: "", events: [], running: false, sidebarMode: "expanded", sidebarWidth: 272, contextWidth: 420 }));

  it("assembles streamed model text in order", () => {
    useAppStore.getState().appendEvent({ type: "model_delta", step: 1, event: { type: "text_delta", text: "hello " } });
    useAppStore.getState().appendEvent({ type: "model_delta", step: 1, event: { type: "text_delta", text: "world" } });
    expect(useAppStore.getState().streamedAnswer).toBe("hello world");
  });

  it("cycles all persisted sidebar states", () => {
    useAppStore.getState().cycleSidebar();
    expect(useAppStore.getState().sidebarMode).toBe("compact");
    useAppStore.getState().cycleSidebar();
    expect(useAppStore.getState().sidebarMode).toBe("hidden");
    useAppStore.getState().cycleSidebar();
    expect(useAppStore.getState().sidebarMode).toBe("expanded");
  });

  it("keeps resizable panels inside usable desktop bounds", () => {
    useAppStore.getState().setSidebarWidth(10);
    useAppStore.getState().setContextWidth(5_000);
    expect(useAppStore.getState().sidebarWidth).toBe(232);
    expect(useAppStore.getState().contextWidth).toBe(680);
  });
});
