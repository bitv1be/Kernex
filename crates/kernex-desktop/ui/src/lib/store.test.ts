import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./store";

describe("desktop event store", () => {
  beforeEach(() => useAppStore.setState({ streamedAnswer: "", events: [], running: false }));

  it("assembles streamed model text in order", () => {
    useAppStore.getState().appendEvent({ type: "model_delta", step: 1, event: { type: "text_delta", text: "hello " } });
    useAppStore.getState().appendEvent({ type: "model_delta", step: 1, event: { type: "text_delta", text: "world" } });
    expect(useAppStore.getState().streamedAnswer).toBe("hello world");
  });
});
