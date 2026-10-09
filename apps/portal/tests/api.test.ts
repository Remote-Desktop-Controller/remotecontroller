import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const f = vi.hoisted(() => ({
  rpc: vi.fn(),
  rate: vi.fn(),
  user: vi.fn(),
  orders: vi.fn(),
  orderPage: vi.fn(),
  findOrder: vi.fn(),
  signOut: vi.fn(),
  exchangeCode: vi.fn(),
}));
vi.mock("../src/lib/supabase", () => ({
  authenticatedUser: f.user,
  administrator: f.user,
  sessionClient: async () => ({
    auth: { signOut: f.signOut, exchangeCodeForSession: f.exchangeCode },
  }),
}));
vi.mock("../src/lib/store", () => ({
  Store: class {
    rpc = f.rpc;
    rate = f.rate;
    orders = f.orders;
    orderPage = f.orderPage;
    findOrder = f.findOrder;
  },
}));
import * as handlers from "../src/lib/handlers";
import { route } from "../src/lib/http";
const subject = "11111111-1111-4111-8111-111111111111",
  operation = "22222222-2222-4222-8222-222222222222";
function post(
  path: string,
  body: unknown,
  headers: Record<string, string> = {},
) {
  return new Request("https://portal.example" + path, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.stubEnv("PORTAL_PUBLIC_URL", "https://portal.example");
  vi.stubEnv("MCP_SERVICE_ORIGIN", "https://remote.example");
  vi.stubEnv("MCP_SHARED_SECRET", "s".repeat(32));
  vi.stubEnv("REMOTE_SERVICE_AVAILABLE", "false");
  f.user.mockResolvedValue({ id: subject, email: "user@example.com" });
  f.rate.mockResolvedValue(undefined);
  f.orders.mockResolvedValue([]);
});
afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});
describe("commercial HTTP contracts", () => {
  it("authenticates webhook readiness before database or provider access", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    const fetcher = vi.fn();
    vi.stubGlobal("fetch", fetcher);
    const result = await route(handlers.webhookHealth)(new Request("https://portal.example/api/asaas/webhook"), {});
    expect(result.status).toBe(401);
    expect(f.rate).not.toHaveBeenCalled();
    expect(fetcher).not.toHaveBeenCalled();
  });
  it("checks real provider compatibility with a read-only request and returns no payment data", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture-private-key");
    vi.stubEnv("ASAAS_ENV", "production");
    vi.stubEnv("SALES_ENABLED", "false");
    const fetcher = vi.fn<typeof fetch>(async () => Response.json({ data: [{ id: "private-payment", customer: "private-customer" }] }));
    vi.stubGlobal("fetch", fetcher);
    const result = await route(handlers.webhookHealth)(new Request("https://portal.example/api/asaas/webhook", { headers: { "asaas-access-token": "w".repeat(32) } }), {});
    expect(result.status).toBe(200);
    expect(await result.json()).toEqual({ ready: true, database: "ok", asaas: "ok", sandbox: false, salesEnabled: false, serviceAvailable: false });
    expect(fetcher.mock.calls).toHaveLength(1);
    expect(fetcher.mock.calls[0][0]).toBe("https://api.asaas.com/v3/payments?limit=1");
    expect(fetcher.mock.calls[0][1]?.method).toBe("GET");
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it.each([
    { event: "PAYMENT_RECEIVED", status: "RECEIVED", expected: "RECEIVED" },
    { event: "PAYMENT_CONFIRMED", status: "CONFIRMED", expected: "CONFIRMED" },
    { event: "PAYMENT_REFUNDED", status: "REFUNDED", expected: "REFUNDED" },
    { event: "PAYMENT_PARTIALLY_REFUNDED", status: "RECEIVED", refunds: [{ status: "DONE", value: 1 }], expected: "REFUND_PARTIALLY_DONE" },
    { event: "PAYMENT_PARTIALLY_REFUNDED", status: "PARTIALLY_REFUNDED", expected: "REFUND_PARTIALLY_DONE" },
    { event: "PAYMENT_DELETED", status: "PENDING", deleted: true, expected: "DELETED" },
    { event: "PAYMENT_RECEIVED", status: "PENDING", expected: "PENDING" },
  ])("reconciles $event using authoritative API state $status", async ({ event, status, expected, ...flags }) => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture");
    const order = { id: operation, user_id: subject, payment_id: "pay_known", customer_id: "cus_known", amount_cents: 2900 };
    f.findOrder.mockResolvedValue(order);
    vi.stubGlobal("fetch", vi.fn(async () => Response.json({ id: "pay_known", externalReference: operation, customer: "cus_known", value: 29, billingType: "PIX", status, ...flags })));
    const request = () => post("/api/asaas/webhook", { id: "evt_fixture", event, payment: { id: "pay_known", status: "RECEIVED", value: 999 } }, { "asaas-access-token": "w".repeat(32) });
    expect((await route(handlers.webhook)(request(), {})).status).toBe(200);
    expect((await route(handlers.webhook)(request(), {})).status).toBe(200);
    expect(f.rpc).toHaveBeenCalledWith("portal_payment_apply", { p_order: operation, p_event: "webhook:evt_fixture", p_status: expected });
    expect(vi.mocked(fetch).mock.calls.every(([, options]) => options?.method === "GET")).toBe(true);
    expect(f.rpc.mock.calls.every(([name]) => name !== "portal_authorize")).toBe(true);
  });
  it("rejects cross-origin checkout before authentication or provider I/O", async () => {
    const result = await route(handlers.checkout)(
      post("/api/checkout", {}, { origin: "https://evil.example" }),
      {},
    );
    expect(result.status).toBe(403);
    expect(f.user).not.toHaveBeenCalled();
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("logout reports provider failure instead of claiming the session ended", async () => {
    f.signOut.mockResolvedValue({ error: new Error("provider unavailable") });
    const result = await route(handlers.logout)(
      post("/api/auth/logout", {}, { origin: "https://portal.example" }),
      {},
    );
    expect(result.status).toBe(503);
  });
  it("refuses client-selected user or amount", async () => {
    const result = await route(handlers.checkout)(
      post(
        "/api/checkout",
        {
          name: "Maria",
          cpfCnpj: "12345678901",
          termsAccepted: true,
          userId: operation,
          amountCents: 1,
        },
        { origin: "https://portal.example" },
      ),
      {},
    );
    expect(result.status).toBe(400);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("requires service secret for MCP including inspect", async () => {
    const result = await route(handlers.authorize)(
      post("/api/mcp/authorize", {
        subject,
        operation_id: operation,
        mode: "inspect",
      }),
      {},
    );
    expect(result.status).toBe(401);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("passes authenticated execute to SQL and service-unavailable cannot be overridden by client", async () => {
    f.rpc.mockResolvedValue({
      subject,
      operation_id: operation,
      allowed: false,
      state: "PAID_AWAITING_SERVICE",
      expires_at: null,
    });
    const result = await route(handlers.authorize)(
      post(
        "/api/mcp/authorize",
        { subject, operation_id: operation, mode: "execute" },
        { authorization: "Bearer " + "s".repeat(32) },
      ),
      {},
    );
    expect(result.status).toBe(200);
    expect(await result.json()).toEqual({
      subject,
      operation_id: operation,
      allowed: false,
      state: "PAID_AWAITING_SERVICE",
      expires_at: null,
    });
    expect(f.rpc).toHaveBeenCalledWith("portal_authorize", {
      p_user: subject,
      p_operation: operation,
      p_mode: "execute",
      p_available: false,
    });
  });
  it("connect only mints ticket and never invokes activation", async () => {
    const r = new Request(
      "https://portal.example/api/mcp/connect?state=abcdefghijklmnop&redirect_uri=" +
        encodeURIComponent("https://remote.example/auth/commercial/callback"),
    );
    const result = await route(handlers.connect)(r, {});
    expect(result.status).toBe(200);
    const body = await result.json();
    expect(new URL(body.redirectUrl).origin).toBe("https://remote.example");
    expect(f.rpc).toHaveBeenCalledTimes(1);
    expect(f.rpc.mock.calls[0][0]).toBe("portal_ticket_create");
    expect(f.rpc.mock.calls[0][1].p_user).toBe(subject);
    expect(f.rpc.mock.calls[0][1].p_hash).toHaveLength(64);
  });
  it("rejects redirect audience forgery before minting ticket", async () => {
    const result = await route(handlers.connect)(
      new Request(
        "https://portal.example/api/mcp/connect?state=abcdefghijklmnop&redirect_uri=https://evil.example/callback",
      ),
      {},
    );
    expect(result.status).toBe(400);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("rejects webhook without its independent token and does not authenticate browser session", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    const result = await route(handlers.webhook)(
      post("/api/asaas/webhook", {}),
      {},
    );
    expect(result.status).toBe(401);
    expect(f.user).not.toHaveBeenCalled();
  });
  it("history reports entitlement-derived expiry and exact period timestamps", async () => {
    const period = {
      activated_at: "2020-01-01T00:00:00Z",
      expires_at: "2020-01-31T00:00:00Z",
      revoked_at: null,
    };
    f.orders.mockResolvedValue([
      {
        id: operation,
        user_id: subject,
        amount_cents: 2900,
        status: "ACTIVE",
        created_at: "2020-01-01T00:00:00Z",
        payment_url: null,
        entitlement: period,
      },
    ]);
    f.rpc.mockResolvedValue({ state: "EXPIRED", pendingDays: 0 });
    const result = await route(handlers.account)(
      new Request("https://portal.example/api/account"),
      {},
    );
    const body = await result.json();
    expect(body.orders[0]).toMatchObject({
      status: "EXPIRED",
      activatedAt: period.activated_at,
      expiresAt: period.expires_at,
    });
  });
  it("admin paginates orders and independently queries older refund requests", async () => {
    f.orderPage.mockImplementation(async (options: { refundOnly?: boolean }) =>
      options.refundOnly
        ? {
            orders: [
              {
                id: operation,
                user_id: subject,
                status: "ACTIVE",
                amount_cents: 2900,
                refund_state: "REQUESTED",
                created_at: "2020-01-01T00:00:00Z",
              },
            ],
            total: 200,
          }
        : { orders: [], total: 500 },
    );
    const result = await route(handlers.admin)(
      new Request(
        "https://portal.example/api/admin?offset=100&limit=50&refundOffset=0&refundLimit=25",
      ),
      {},
    );
    const body = await result.json();
    expect(body.orderPagination).toEqual({
      offset: 100,
      limit: 50,
      total: 500,
      hasMore: true,
    });
    expect(body.refundPagination).toEqual({
      offset: 0,
      limit: 25,
      total: 200,
      hasMore: true,
    });
    expect(body.refundRequests[0].orderId).toBe(operation);
    expect(f.orderPage).toHaveBeenCalledWith({
      offset: 100,
      limit: 50,
      refundOnly: false,
    });
    expect(f.orderPage).toHaveBeenCalledWith({
      offset: 0,
      limit: 25,
      refundOnly: true,
    });
  });
  it("authenticated unrelated Asaas campaign with malformed external reference is acknowledged without credit", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json({
          id: "pay_other",
          externalReference: "blog-campaign",
          customer: "cus_other",
          value: 10,
          billingType: "PIX",
          status: "RECEIVED",
        }),
      ),
    );
    const result = await route(handlers.webhook)(
      post(
        "/api/asaas/webhook",
        {
          id: "evt",
          event: "PAYMENT_RECEIVED",
          payment: { id: "pay_other", externalReference: "blog-campaign" },
        },
        { "asaas-access-token": "w".repeat(32) },
      ),
      {},
    );
    expect(result.status).toBe(200);
    expect(f.rpc).not.toHaveBeenCalled();
    expect(f.findOrder).not.toHaveBeenCalled();
  });
  it("source payment with a UUID outside this portal campaign is acknowledged without credit", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture");
    f.findOrder.mockResolvedValue(null);
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json({
          id: "pay_other",
          externalReference: operation,
          customer: "cus_other",
          value: 10,
          billingType: "PIX",
          status: "RECEIVED",
        }),
      ),
    );
    const result = await route(handlers.webhook)(
      post(
        "/api/asaas/webhook",
        { id: "evt", event: "PAYMENT_RECEIVED", payment: { id: "pay_other" } },
        { "asaas-access-token": "w".repeat(32) },
      ),
      {},
    );
    expect(result.status).toBe(200);
    expect(f.findOrder).toHaveBeenCalledWith(operation);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("known payment with forged payload reference fails closed using provider reference", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture");
    f.findOrder.mockResolvedValue({
      id: operation,
      user_id: subject,
      payment_id: "pay_known",
      customer_id: "cus_known",
      amount_cents: 2900,
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json({
          id: "pay_known",
          externalReference: operation,
          customer: "cus_known",
          value: 29,
          billingType: "PIX",
          status: "RECEIVED",
        }),
      ),
    );
    const result = await route(handlers.webhook)(
      post(
        "/api/asaas/webhook",
        {
          id: "evt",
          event: "PAYMENT_RECEIVED",
          payment: { id: "pay_known", externalReference: subject },
        },
        { "asaas-access-token": "w".repeat(32) },
      ),
      {},
    );
    expect(result.status).toBe(409);
    expect(f.findOrder).toHaveBeenCalledWith(operation);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("webhook provider outage is retryable rather than acknowledged", async () => {
    vi.stubEnv("ASAAS_WEBHOOK_TOKEN", "w".repeat(32));
    vi.stubEnv("ASAAS_API_KEY", "fixture");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 503 })),
    );
    const result = await route(handlers.webhook)(
      post(
        "/api/asaas/webhook",
        { id: "evt", event: "PAYMENT_RECEIVED", payment: { id: "pay_known" } },
        { "asaas-access-token": "w".repeat(32) },
      ),
      {},
    );
    expect(result.status).toBe(503);
    expect(f.rpc).not.toHaveBeenCalled();
  });
  it("auth callback uses provider PKCE verification and safe local redirect", async () => {
    f.exchangeCode.mockResolvedValue({ error: null });
    const result = await route(handlers.callback)(
      new Request(
        "https://portal.example/auth/callback?code=fixture&next=" +
          encodeURIComponent("/\t/evil.example"),
      ),
      {},
    );
    expect(f.exchangeCode).toHaveBeenCalledWith("fixture");
    expect(result.headers.get("location")).toBe(
      "https://portal.example/dashboard",
    );
  });
});
