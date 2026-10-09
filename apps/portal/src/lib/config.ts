import "server-only";
const text = (key: string) => process.env[key]?.trim() || null;
function origin(value: string | null, allowLoopback = false) {
  if (!value) return null;
  try {
    const u = new URL(value);
    const protocol =
      u.protocol === "https:" ||
      (allowLoopback &&
        u.protocol === "http:" &&
        ["localhost", "127.0.0.1"].includes(u.hostname));
    return protocol &&
      !u.username &&
      !u.password &&
      !u.search &&
      !u.hash &&
      u.pathname === "/"
      ? u.origin
      : null;
  } catch {
    return null;
  }
}
function download(value: string | null) {
  if (!value) return null;
  try {
    const u = new URL(value);
    return u.protocol === "https:" && !u.username && !u.password
      ? u.href
      : null;
  } catch {
    return null;
  }
}
export function getConfig() {
  const priceCents = 990;
  const environment = text("ASAAS_ENV") ?? "sandbox";
  const environmentValid = ["sandbox", "production"].includes(environment);
  const sandbox = environment !== "production";
  const deliveryDeadline = text("DELIVERY_DEADLINE"),
    supportEmail = text("SUPPORT_EMAIL"),
    operatorName = text("LEGAL_OPERATOR_NAME"),
    refundPolicy = text("REFUND_POLICY");
  const remoteOrigin = origin(text("MCP_SERVICE_ORIGIN")),
    mcpSecret = text("MCP_SHARED_SECRET");
  const serviceAvailable =
    text("REMOTE_SERVICE_AVAILABLE") === "true" &&
    !!remoteOrigin &&
    !!mcpSecret &&
    mcpSecret.length >= 32;
  const deadlineMillis = deliveryDeadline ? Date.parse(deliveryDeadline) : NaN;
  const deadlineValid =
    !!deliveryDeadline &&
    /^\d{4}-\d{2}-\d{2}$/.test(deliveryDeadline) &&
    Number.isFinite(deadlineMillis) &&
    new Date(deadlineMillis).toISOString().slice(0, 10) === deliveryDeadline;
  const commercialComplete = Boolean(
    deadlineValid &&
      (serviceAvailable || deadlineMillis > Date.now()) &&
      supportEmail &&
      /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(supportEmail) &&
      operatorName &&
      refundPolicy,
  );
  return {
    priceCents,
    freeDailyEnabled: text("FREE_DAILY_ENABLED") === "true",
    sandbox,
    salesEnabled:
      environmentValid &&
      text("SALES_ENABLED") === "true" &&
      priceCents !== null &&
      (sandbox || commercialComplete),
    serviceAvailable,
    deliveryDeadline,
    supportEmail,
    operatorName,
    refundPolicy:
      refundPolicy ??
      "As condições de reembolso serão divulgadas antes da abertura das vendas.",
    remoteOrigin,
    agentDownloadUrl: download(text("AGENT_DOWNLOAD_URL")),
    portalOrigin: origin(
      text("PORTAL_PUBLIC_URL"),
      text("PORTAL_ALLOW_HTTP_LOOPBACK") === "true",
    ),
    mcpSecret,
    asaasApiKey: text("ASAAS_API_KEY"),
    webhookToken: text("ASAAS_WEBHOOK_TOKEN"),
    supabaseUrl: text("NEXT_PUBLIC_SUPABASE_URL"),
    supabaseAnonKey: text("NEXT_PUBLIC_SUPABASE_ANON_KEY"),
    supabaseServiceKey: text("SUPABASE_SERVICE_ROLE_KEY"),
    adminUserIds: (text("ADMIN_USER_IDS") ?? "")
      .split(",")
      .map((s) => s.trim())
      .filter((s) =>
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
          s,
        ),
      ),
  };
}
export function getPublicConfig() {
  const c = getConfig();
  return {
    priceCents: c.priceCents,
    freeDailyEnabled: c.freeDailyEnabled,
    salesEnabled: c.salesEnabled,
    sandbox: c.sandbox,
    serviceAvailable: c.serviceAvailable,
    deliveryDeadline: c.deliveryDeadline,
    supportEmail: c.supportEmail,
    operatorName: c.operatorName,
    refundPolicy: c.refundPolicy,
    remoteOrigin: c.remoteOrigin,
    agentDownloadUrl: c.agentDownloadUrl,
  };
}
export type Config = ReturnType<typeof getConfig>;
