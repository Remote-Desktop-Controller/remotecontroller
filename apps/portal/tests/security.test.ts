import { describe, it, expect, vi, afterEach } from "vitest";
import { getPublicConfig, getConfig } from "../src/lib/config";
import { Asaas, validatePayment } from "../src/lib/asaas";
afterEach(() => vi.unstubAllEnvs());
describe("server commercial boundary", () => {
  it("blocks production sales until all commercial conditions exist and never serializes secrets", () => {
    vi.stubEnv("ASAAS_ENV", "production");
    vi.stubEnv("SALES_ENABLED", "true");
    vi.stubEnv("PORTAL_PRICE_CENTS", "2990");
    vi.stubEnv("ASAAS_API_KEY", "secret-value");
    expect(getPublicConfig().salesEnabled).toBe(false);
    vi.stubEnv("DELIVERY_DEADLINE", "2026-12-01");
    vi.stubEnv("SUPPORT_EMAIL", "support@example.com");
    vi.stubEnv("LEGAL_OPERATOR_NAME", "Remote Controller");
    vi.stubEnv("REFUND_POLICY", "Reembolso via suporte.");
    expect(getPublicConfig().salesEnabled).toBe(true);
    expect(JSON.stringify(getPublicConfig())).not.toContain("secret-value");
  });
  it("keeps the fixed Premium price independent of stale environment and requires sales flag", () => {
    vi.stubEnv("ASAAS_ENV", "sandbox");
    vi.stubEnv("SALES_ENABLED", "true");
    vi.stubEnv("PORTAL_PRICE_CENTS", "");
    expect(getPublicConfig().priceCents).toBe(990);
    expect(getPublicConfig().salesEnabled).toBe(true);
    vi.stubEnv("PORTAL_PRICE_CENTS", "2990");
    expect(getPublicConfig().salesEnabled).toBe(true);
    vi.stubEnv("SALES_ENABLED", "false");
    expect(getPublicConfig().salesEnabled).toBe(false);
  });
  it("disables remote availability without authenticated configured service", () => {
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE", "true");
    vi.stubEnv("MCP_SERVICE_ORIGIN", "https://remote.example/path");
    expect(getPublicConfig().serviceAvailable).toBe(false);
    vi.stubEnv("MCP_SERVICE_ORIGIN", "https://remote.example");
    vi.stubEnv("MCP_SHARED_SECRET", "x".repeat(32));
    expect(getPublicConfig().serviceAvailable).toBe(true);
  });
  it("requires explicit opt-in for HTTP localhost and rejects arbitrary HTTP origins", () => {
    vi.stubEnv("PORTAL_PUBLIC_URL", "http://localhost:3000");
    vi.stubEnv("PORTAL_ALLOW_HTTP_LOOPBACK", "false");
    expect(getConfig().portalOrigin).toBeNull();
    vi.stubEnv("PORTAL_ALLOW_HTTP_LOOPBACK", "true");
    expect(getConfig().portalOrigin).toBe("http://localhost:3000");
    for (const url of [
      "http://evil.example",
      "http://localhost.evil.example",
      "http://user@localhost:3000",
      "http://localhost:3000/path",
      "http://localhost:3000?secret=1",
    ]) {
      vi.stubEnv("PORTAL_PUBLIC_URL", url);
      expect(getConfig().portalOrigin).toBeNull();
    }
  });
  it("invalid Asaas environment never selects real payment API or enables sales", () => {
    vi.stubEnv("ASAAS_ENV", "produciton");
    vi.stubEnv("SALES_ENABLED", "true");
    vi.stubEnv("PORTAL_PRICE_CENTS", "2990");
    vi.stubEnv("DELIVERY_DEADLINE", "2099-12-01");
    vi.stubEnv("SUPPORT_EMAIL", "support@example.com");
    vi.stubEnv("LEGAL_OPERATOR_NAME", "Remote Controller");
    vi.stubEnv("REFUND_POLICY", "Refund policy");
    expect(getPublicConfig().salesEnabled).toBe(false);
    expect(getConfig().sandbox).toBe(true);
  });
  it("production presale requires a real future date, but delivered service allows historical deadline", () => {
    vi.stubEnv("ASAAS_ENV", "production");
    vi.stubEnv("SALES_ENABLED", "true");
    vi.stubEnv("PORTAL_PRICE_CENTS", "2990");
    vi.stubEnv("SUPPORT_EMAIL", "support@example.com");
    vi.stubEnv("LEGAL_OPERATOR_NAME", "Remote Controller");
    vi.stubEnv("REFUND_POLICY", "Refund policy");
    vi.stubEnv("DELIVERY_DEADLINE", "2099-02-31");
    expect(getPublicConfig().salesEnabled).toBe(false);
    vi.stubEnv("DELIVERY_DEADLINE", "2020-01-01");
    expect(getPublicConfig().salesEnabled).toBe(false);
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE", "true");
    vi.stubEnv("MCP_SERVICE_ORIGIN", "https://remote.example");
    vi.stubEnv("MCP_SHARED_SECRET", "s".repeat(32));
    expect(getPublicConfig().salesEnabled).toBe(true);
  });
  it("only exposes credential-free HTTPS agent downloads", () => {
    for (const value of [
      "javascript:alert(1)",
      "http://downloads.example/agent.exe",
      "https://secret@downloads.example/agent.exe",
    ]) {
      vi.stubEnv("AGENT_DOWNLOAD_URL", value);
      expect(getPublicConfig().agentDownloadUrl).toBeNull();
    }
    vi.stubEnv("AGENT_DOWNLOAD_URL", "https://downloads.example/agent.exe");
    expect(getPublicConfig().agentDownloadUrl).toBe(
      "https://downloads.example/agent.exe",
    );
  });
  it("rejects mismatched provider customer, reference, cents or method", () => {
    const order = {
      id: "order",
      customer_id: "customer",
      amount_cents: 2990,
      payment_id: "payment",
    };
    const payment = {
      id: "payment",
      customer: "customer",
      externalReference: "order",
      value: 29.9,
      billingType: "PIX",
      status: "RECEIVED",
    };
    expect(() => validatePayment(order, payment)).not.toThrow();
    for (const change of [
      { customer: "other" },
      { externalReference: "other" },
      { value: 29.91 },
      { billingType: "CREDIT_CARD" },
      { id: "other" },
    ])
      expect(() => validatePayment(order, { ...payment, ...change })).toThrow();
  });
  it("uses real one-time PIX HTTP requests, configured callback, query reconciliation and injectable transport", async () => {
    vi.stubEnv("ASAAS_ENV", "sandbox");
    vi.stubEnv("ASAAS_API_KEY", "key");
    vi.stubEnv("PORTAL_PUBLIC_URL", "https://portal.example");
    const calls: { url: string; init: RequestInit }[] = [];
    const client = new Asaas(getConfig(), async (input, init) => {
      calls.push({ url: String(input), init: init ?? {} });
      return Response.json({ id: "pay", data: [] });
    });
    await client.createPayment("customer", "order", 2990);
    await client.findPayment("order");
    expect(calls[0].url).toBe("https://api-sandbox.asaas.com/v3/payments");
    const payload = JSON.parse(String(calls[0].init.body));
    expect(payload.billingType).toBe("PIX");
    expect(payload.value).toBe(29.9);
    expect(payload.externalReference).toBe("order");
    expect(payload.subscription).toBeUndefined();
    expect(payload.callback).toEqual({
      successUrl: "https://portal.example/purchase?orderId=order",
      autoRedirect: true,
    });
    expect(calls[1].url).toContain("externalReference=order");
  });
});
