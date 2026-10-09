import "server-only";
import type { Config } from "./config";
import type { Store } from "./store";
import type { Asaas } from "./asaas";
import { fail, secureEqual } from "./http";

const requiredEvents = [
  "PAYMENT_RECEIVED", "PAYMENT_CONFIRMED", "PAYMENT_REFUNDED",
  "PAYMENT_PARTIALLY_REFUNDED", "PAYMENT_DELETED",
];

// Aggregate reads only. Never return provider records, identifiers or credentials.
export async function webhookDiagnostics(c: Config, store: Store, provider: Asaas) {
  async function count(table: string) {
    const result = await store.db.from(table).select("id", { count: "exact", head: true });
    if (result.error || result.count === null) fail(503, "DATABASE", "Contagem não confirmada.");
    return result.count;
  }
  const [orders, entitlements, events, operations, rate] = await Promise.all([
    count("portal_orders"), count("portal_entitlements"), count("portal_events"), count("portal_operations"),
    store.db.from("portal_rate_limits").select("hits,starts_at").eq("key", "webhook").maybeSingle(),
  ]);
  if (rate.error) fail(503, "DATABASE", "Contagem de recebimentos não confirmada.");
  const page = await provider.request<{ data: Record<string, unknown>[]; hasMore?: boolean }>("/webhooks?limit=100&offset=0");
  if (!Array.isArray(page.data)) fail(503, "PROVIDER_RESPONSE", "Configuração Asaas não confirmada.");
  const endpoint = c.portalOrigin ? c.portalOrigin + "/api/asaas/webhook" : null;
  const matches = page.data.filter(h => h.url === endpoint || h.name === "Remote Controller - Pagamentos").map(h => {
    const configuredToken = typeof h.authToken === "string" && h.authToken.length > 0 && !h.authToken.includes("*") ? h.authToken : null;
    const configuredEvents = Array.isArray(h.events) ? h.events : [];
    return {
      urlMatches: h.url === endpoint,
      enabled: typeof h.enabled === "boolean" ? h.enabled : null,
      apiVersion: String(h.apiVersion) === "3" ? 3 : null,
      sendType: ["SEQUENTIALLY", "NON_SEQUENTIALLY"].includes(String(h.sendType)) ? h.sendType : "UNKNOWN",
      queueActive: typeof h.interrupted === "boolean" ? !h.interrupted : null,
      tokenMatches: configuredToken ? secureEqual(configuredToken, c.webhookToken) : null,
      missingEvents: requiredEvents.filter(e => !configuredEvents.includes(e)),
    };
  });
  return {
    scope: "configuration-and-database-read-only",
    database: { orders, entitlements, events, operations, webhookRate: rate.data ? { hits: rate.data.hits, startsAt: rate.data.starts_at } : null },
    webhooks: { matches, configurationPageHasMore: page.hasMore === true },
    providerDeliveryProven: false,
  };
}
