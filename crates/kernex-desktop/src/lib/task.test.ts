import { describe, expect, it } from "vitest";
import { prepareTask } from "./task";

describe("composer task preparation", () => {
  it("keeps a normal agent request unchanged", () => {
    expect(prepareTask({ task: "inspect the parser", attachments: [], mode: "agent" })).toBe("inspect the parser");
  });

  it("passes workspace attachments as explicit file references", () => {
    expect(prepareTask({ task: "review these", attachments: ["src/main.rs", "Cargo.toml"], mode: "agent" })).toContain("[Kernex attachments]\n- src/main.rs\n- Cargo.toml\n[/Kernex attachments]\n\nreview these");
  });

  it("makes plan and review modes behaviorally explicit", () => {
    expect(prepareTask({ task: "upgrade", attachments: [], mode: "plan" })).toContain("without modifying files");
    expect(prepareTask({ task: "upgrade", attachments: [], mode: "review" })).toContain("prioritize concrete findings");
  });
});
