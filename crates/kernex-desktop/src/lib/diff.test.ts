import { describe, expect, it } from "vitest";
import { parseDiff } from "./diff";

describe("file change summaries", () => {
  it("identifies modified files and line counts", () => {
    expect(parseDiff("--- a/src/lib.rs\n+++ b/src/lib.rs\n-old\n+new\n+more")).toEqual({ path: "src/lib.rs", status: "modified", added: 2, removed: 1 });
  });

  it("identifies created and deleted files", () => {
    expect(parseDiff("--- /dev/null\n+++ b/new.txt\n+hello").status).toBe("created");
    expect(parseDiff("--- a/old.txt\n+++ /dev/null\n-goodbye").status).toBe("deleted");
  });

  it("surfaces conflicted files without relying on color", () => {
    expect(parseDiff("--- a/file\n+++ b/file\n<<<<<<< ours\n+a\n=======\n-b\n>>>>>>> theirs").status).toBe("conflicted");
  });
});
