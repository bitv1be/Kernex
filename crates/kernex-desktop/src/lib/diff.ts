export interface ParsedDiff {
  path: string;
  status: "created" | "modified" | "deleted" | "renamed" | "unchanged" | "conflicted";
  added: number;
  removed: number;
}

export function parseDiff(diff: string): ParsedDiff {
  const newPath = diff.match(/^\+\+\+\s+(?:b\/)?(.+)$/m)?.[1];
  const oldPath = diff.match(/^---\s+(?:a\/)?(.+)$/m)?.[1];
  const path = newPath && newPath !== "/dev/null" ? newPath : oldPath && oldPath !== "/dev/null" ? oldPath : "Working tree changes";
  const added = diff.split("\n").filter((line) => line.startsWith("+") && !line.startsWith("+++")).length;
  const removed = diff.split("\n").filter((line) => line.startsWith("-") && !line.startsWith("---")).length;
  const status = oldPath === "/dev/null" ? "created" : newPath === "/dev/null" ? "deleted" : /rename from|rename to/m.test(diff) ? "renamed" : /<<<<<<<|>>>>>>>/m.test(diff) ? "conflicted" : added || removed ? "modified" : "unchanged";
  return { path, status, added, removed };
}
