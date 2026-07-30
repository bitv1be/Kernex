// @vitest-environment jsdom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { CodexAuthPanel } from "./settings-sheet";

beforeAll(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

afterEach(() => {
  document.body.replaceChildren();
});

function renderPanel(properties: React.ComponentProps<typeof CodexAuthPanel>) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<CodexAuthPanel {...properties} />));
  return { container, root };
}

describe("Codex desktop authentication", () => {
  it("explains and starts managed ChatGPT OAuth from the disconnected state", () => {
    const onLogin = vi.fn();
    const { container, root } = renderPanel({
      phase: "idle",
      onLogin,
      onLogout: vi.fn(),
      onRefresh: vi.fn(),
    });

    const login = Array.from(container.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("Continue with ChatGPT"));
    expect(login).toBeDefined();
    expect(container.textContent).toContain("no API key setup required");
    expect(container.textContent).toContain("default browser");
    act(() => login?.click());
    expect(onLogin).toHaveBeenCalledOnce();
    act(() => root.unmount());
  });

  it("renders the connected plan, usage meter, refresh, and sign-out actions", () => {
    const onLogout = vi.fn();
    const onRefresh = vi.fn();
    const { container, root } = renderPanel({
      account: {
        account: { type: "chatgpt", email: "person@example.com", planType: "plus" },
        requiresOpenaiAuth: true,
      },
      limits: {
        rateLimits: { primary: { usedPercent: 25, windowDurationMins: 300 } },
        rateLimitResetCredits: { availableCount: 2 },
      },
      phase: "idle",
      onLogin: vi.fn(),
      onLogout,
      onRefresh,
    });

    expect(container.textContent).toContain("Connected");
    expect(container.textContent).toContain("person@example.com");
    expect(container.textContent).toContain("ChatGPT Plus");
    expect(container.textContent).toContain("25% used");
    expect(container.textContent).toContain("300-minute window");
    expect(container.textContent).toContain("2 resets available");
    const meter = container.querySelector<HTMLElement>("[role=progressbar]");
    expect(meter?.getAttribute("aria-valuenow")).toBe("25");
    const refresh = container.querySelector<HTMLButtonElement>("[aria-label='Refresh ChatGPT account']");
    const logout = Array.from(container.querySelectorAll("button")).find((button) => button.textContent?.includes("Sign out"));
    act(() => refresh?.click());
    act(() => logout?.click());
    expect(onRefresh).toHaveBeenCalledOnce();
    expect(onLogout).toHaveBeenCalledOnce();
    act(() => root.unmount());
  });

  it("shows distinct loading and browser callback states", () => {
    const loading = renderPanel({
      phase: "loading",
      onLogin: vi.fn(),
      onLogout: vi.fn(),
      onRefresh: vi.fn(),
    });
    expect(loading.container.textContent).toContain("Checking your ChatGPT connection");
    expect(loading.container.querySelector("[aria-busy=true]")).not.toBeNull();
    act(() => loading.root.unmount());

    const waiting = renderPanel({
      phase: "signing-in",
      onLogin: vi.fn(),
      onLogout: vi.fn(),
      onRefresh: vi.fn(),
    });
    expect(waiting.container.textContent).toContain("Waiting for ChatGPT…");
    expect(waiting.container.textContent).toContain("Complete sign-in in your browser");
    expect(waiting.container.textContent).toContain("update automatically");
    expect(waiting.container.querySelector("button")?.disabled).toBe(true);
    act(() => waiting.root.unmount());
  });

  it("surfaces login failures without hiding the retry action", () => {
    const failed = renderPanel({
      phase: "idle",
      error: "Codex App Server is not available",
      onLogin: vi.fn(),
      onLogout: vi.fn(),
      onRefresh: vi.fn(),
    });
    expect(failed.container.textContent).toContain("Couldn’t connect to ChatGPT");
    expect(failed.container.textContent).toContain("Codex App Server is not available");
    expect(failed.container.textContent).toContain("Continue with ChatGPT");
    act(() => failed.root.unmount());
  });

  it("offers ChatGPT login when Codex currently uses an API key", () => {
    const apiKey = renderPanel({
      account: { account: { type: "apiKey" }, requiresOpenaiAuth: true },
      phase: "idle",
      onLogin: vi.fn(),
      onLogout: vi.fn(),
      onRefresh: vi.fn(),
    });
    expect(apiKey.container.textContent).toContain("API key active");
    expect(apiKey.container.textContent).toContain("switch to your subscription-backed account");
    expect(apiKey.container.textContent).toContain("Continue with ChatGPT");
    act(() => apiKey.root.unmount());
  });
});
