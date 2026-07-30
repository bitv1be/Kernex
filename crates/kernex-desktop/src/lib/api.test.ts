import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { api } from "./api";

describe("Codex desktop command bridge", () => {
  beforeEach(() => mocks.invoke.mockReset().mockResolvedValue(undefined));

  it("maps account, login, logout, and limits to registered Tauri commands", async () => {
    await api.codexAccount();
    await api.codexLogin();
    await api.codexLogout();
    await api.codexRateLimits();

    expect(mocks.invoke.mock.calls).toEqual([
      ["codex_account"],
      ["codex_login"],
      ["codex_sign_out"],
      ["codex_limits"],
    ]);
  });

  it("propagates managed OAuth failures to the desktop UI", async () => {
    mocks.invoke.mockRejectedValueOnce(new Error("browser callback failed"));
    await expect(api.codexLogin()).rejects.toThrow("browser callback failed");
  });
});
