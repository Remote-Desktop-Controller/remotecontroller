import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const f = vi.hoisted(() => ({
  getUser: vi.fn(),
  setCookie: vi.fn(),
  create: vi.fn(),
}));
vi.mock("@supabase/ssr", () => ({ createServerClient: f.create }));
vi.mock("next/headers", () => ({
  cookies: async () => ({ getAll: () => [], set: f.setCookie }),
}));
import {
  authenticatedUser,
  administrator,
  sessionClient,
} from "../src/lib/supabase";
const id = "11111111-1111-4111-8111-111111111111";
beforeEach(() => {
  vi.resetAllMocks();
  vi.stubEnv("NEXT_PUBLIC_SUPABASE_URL", "https://project.supabase.co");
  vi.stubEnv("NEXT_PUBLIC_SUPABASE_ANON_KEY", "public-key");
  vi.stubEnv("PORTAL_PUBLIC_URL", "https://portal.example");
  f.create.mockReturnValue({ auth: { getUser: f.getUser } });
});
afterEach(() => vi.unstubAllEnvs());
describe("verified server identity", () => {
  it("requires fresh Supabase validated identity rather than trusting cookie payload", async () => {
    f.getUser.mockResolvedValue({
      data: { user: null },
      error: new Error("invalid token"),
    });
    await expect(authenticatedUser()).rejects.toMatchObject({ status: 401 });
    expect(f.getUser).toHaveBeenCalledOnce();
  });
  it("rejects an authenticated identity until email is confirmed", async () => {
    f.getUser.mockResolvedValue({
      data: {
        user: { id, email: "user@example.com", email_confirmed_at: null },
      },
      error: null,
    });
    await expect(authenticatedUser()).rejects.toMatchObject({ status: 403 });
  });
  it("admin authority exclusively checks verified UUID server allowlist", async () => {
    f.getUser.mockResolvedValue({
      data: {
        user: {
          id,
          email: "admin@example.com",
          email_confirmed_at: "2026-10-08T00:00:00Z",
        },
      },
      error: null,
    });
    vi.stubEnv("ADMIN_USER_IDS", "admin@example.com");
    await expect(administrator()).rejects.toMatchObject({ status: 403 });
    vi.stubEnv("ADMIN_USER_IDS", id);
    expect((await administrator()).id).toBe(id);
  });
  it("sets Supabase session cookies HttpOnly, Secure and SameSite=Lax", async () => {
    await sessionClient();
    const options = f.create.mock.calls[0][2];
    options.cookies.setAll([
      { name: "sb-session", value: "fixture", options: { path: "/" } },
    ]);
    expect(f.setCookie).toHaveBeenCalledWith("sb-session", "fixture", {
      path: "/",
      httpOnly: true,
      secure: true,
      sameSite: "lax",
    });
  });
});
