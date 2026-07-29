import type { ComposerSubmission } from "@/components/composer/message-composer";

export function prepareTask(submission: ComposerSubmission) {
  const attachments = submission.attachments.length ? `[Kernex attachments]\n${submission.attachments.map((path) => `- ${path}`).join("\n")}\n[/Kernex attachments]\n\n` : "";
  const mode = submission.mode === "plan" ? "Plan mode: analyze the request and produce a concrete implementation plan without modifying files or running destructive actions.\n\n" : submission.mode === "review" ? "Review mode: inspect the relevant existing code and changes, prioritize concrete findings, and do not modify files unless explicitly asked.\n\n" : "";
  return `${attachments}${mode}${submission.task}`;
}
