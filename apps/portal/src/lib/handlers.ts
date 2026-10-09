import "server-only";
import { randomBytes } from "node:crypto";
import { z } from "zod";
import { getConfig, getPublicConfig } from "./config";
import { authenticatedUser, administrator, sessionClient } from "./supabase";
import { Billing } from "./billing";
import { Asaas } from "./asaas";
import { webhookDiagnostics } from "./webhook-diagnostics";
import { safeNext } from "./auth-session";
import { Store, type Order } from "./store";
import {
  fail,
  jsonBody,
  sameOrigin,
  mcpAuth,
  secureEqual,
  hash,
  uuid,
  response,
  type RouteContext,
  HttpError,
} from "./http";

const credentials = z
  .object({
    email: z.string().email().max(254),
    password: z.string().min(12).max(128),
  })
  .strict();
const loginCredentials = z
  .object({
    email: z.string().email().max(254),
    password: z.string().min(1).max(128),
  })
  .strict();
const summary = (o: Order) => {
  const e = o.entitlement;
  const status = e?.revoked_at
    ? "REFUNDED"
    : e?.activated_at
      ? e.expires_at && Date.parse(e.expires_at) > Date.now()
        ? "ACTIVE"
        : "EXPIRED"
      : e
        ? getConfig().serviceAvailable
          ? "READY_TO_ACTIVATE"
          : "PAID_AWAITING_SERVICE"
        : o.status;
  return {
    id: o.id,
    amountCents: Number(o.amount_cents),
    status,
    paymentUrl: o.payment_url,
    createdAt: o.created_at,
    activatedAt: e?.activated_at ?? null,
    expiresAt: e?.expires_at ?? null,
  };
};
async function userPost(r: Request, key: string) {
  sameOrigin(r);
  const user = await authenticatedUser();
  await new Store().rate(key + ":" + user.id);
  return user;
}
async function orderId(context: RouteContext) {
  if (!context.params) fail(400, "INVALID_ORDER", "Pedido inválido.");
  const { id } = await context.params;
  const parsed = uuid.safeParse(id);
  if (!parsed.success) fail(400, "INVALID_ORDER", "Pedido inválido.");
  return parsed.data;
}
export async function config() {
  return response(getPublicConfig());
}
export async function register(r: Request) {
  sameOrigin(r);
  const body = await jsonBody(r, credentials);
  const store = new Store();
  await store.rate("auth:register", 30);
  await store.rate("register:" + hash(body.email.toLowerCase()), 3, 3600);
  const c = getConfig();
  if (!c.portalOrigin) fail(503, "CONFIGURATION", "Portal não configurado.");
  const client = await sessionClient();
  const { error } = await client.auth.signUp({
    ...body,
    options: { emailRedirectTo: c.portalOrigin + "/auth/callback" },
  });
  if (error)
    fail(
      400,
      "REGISTRATION",
      "Não foi possível cadastrar. Confira o e-mail e a senha.",
    );
  return response({
    message: "Confira seu e-mail para confirmar o cadastro.",
    emailVerificationRequired: true,
  });
}
export async function login(r: Request) {
  sameOrigin(r);
  const body = await jsonBody(r, loginCredentials);
  const store = new Store();
  await store.rate("auth:login", 120);
  await store.rate("login:" + hash(body.email.toLowerCase()), 10, 600);
  const client = await sessionClient();
  const { data, error } = await client.auth.signInWithPassword(body);
  if (error || !data.user)
    fail(
      401,
      "LOGIN_FAILED",
      "E-mail ou senha inválidos, ou e-mail ainda não confirmado.",
    );
  if (!data.user.email_confirmed_at) {
    await client.auth.signOut();
    fail(403, "EMAIL_NOT_VERIFIED", "Confirme o e-mail para entrar.");
  }
  return response({ ok: true });
}
export async function logout(r: Request) {
  sameOrigin(r);
  const client = await sessionClient();
  const { error } = await client.auth.signOut({ scope: "local" });
  if (error)
    fail(
      503,
      "LOGOUT_FAILED",
      "Não foi possível encerrar a sessão. Tente novamente.",
    );
  return response({ ok: true });
}
export async function session() {
  try {
    const user = await authenticatedUser();
    return response({ user: { id: user.id, email: user.email!, emailVerified: true,
      confirmedAt: user.email_confirmed_at! } });
  } catch (error) {
    if (error instanceof HttpError && [401, 403].includes(error.status))
      return response({ user: null });
    throw error;
  }
}
export async function account() {
  const u = await authenticatedUser(),
    s = new Store(),
    c = getConfig();
  await s.rate("account:" + u.id, 60);
  const [subscription, orders] = await Promise.all([
    s.rpc("portal_subscription", {
      p_user: u.id,
      p_available: c.serviceAvailable,
    }),
    s.orders(u.id),
  ]);
  return response({
    user: { id: u.id, email: u.email },
    subscription,
    orders: orders.map(summary),
    serviceAvailable: c.serviceAvailable,
    salesEnabled: c.salesEnabled,
  });
}
export async function checkout(r: Request) {
  const u = await userPost(r, "checkout");
  const body = await jsonBody(
    r,
    z
      .object({
        name: z.string().trim().min(2).max(150),
        cpfCnpj: z.string().regex(/^(?:\d{11}|\d{14})$/),
        termsAccepted: z.literal(true),
      })
      .strict(),
  );
  return response(
    await new Billing().checkout(
      { id: u.id, email: u.email! },
      body.name,
      body.cpfCnpj,
    ),
  );
}
export async function sync(r: Request, ctx: RouteContext) {
  const u = await userPost(r, "sync");
  return response(summary(await new Billing().sync(await orderId(ctx), u.id)));
}
export async function cancel(r: Request, ctx: RouteContext) {
  const u = await userPost(r, "cancel");
  return response(
    summary(await new Billing().cancel(await orderId(ctx), u.id)),
  );
}
export async function refundRequest(r: Request, ctx: RouteContext) {
  const u = await userPost(r, "refund-request");
  return response(await new Billing().requestRefund(await orderId(ctx), u.id));
}
function authenticatedWebhookConfig(r: Request) {
  const c = getConfig();
  if (!c.webhookToken || c.webhookToken.length < 16)
    fail(503, "CONFIGURATION", "Webhook não configurado.");
  if (!secureEqual(r.headers.get("asaas-access-token"), c.webhookToken))
    fail(401, "WEBHOOK_AUTH", "Webhook não autenticado.");
  return c;
}
export async function webhookHealth(r: Request) {
  const c = authenticatedWebhookConfig(r);
  const store = new Store();
  const provider = new Asaas(c);
  await store.rate("webhook-readiness", 10);
  const result = await provider.request<{ data: unknown[] }>(
    "/payments?limit=1",
  );
  if (!Array.isArray(result.data))
    fail(503, "PROVIDER_RESPONSE", "Resposta da operadora não confirmada.");
  return response({
    ready: true,
    database: "ok",
    asaas: "ok",
    sandbox: c.sandbox,
    salesEnabled: c.salesEnabled,
    serviceAvailable: c.serviceAvailable,
    ...(new URL(r.url).searchParams.get("diagnostics") === "1"
      ? { diagnostics: await webhookDiagnostics(c, store, provider) }
      : {}),
  });
}
export async function webhook(r: Request) {
  authenticatedWebhookConfig(r);
  const body = await jsonBody(
    r,
    z
      .object({
        id: z.string().min(1).max(200),
        event: z.string().max(100),
        payment: z
          .object({
            id: z.string().min(1).max(200),
            externalReference: z.unknown().optional(),
          })
          .passthrough()
          .optional(),
      })
      .passthrough(),
  );
  await new Store().rate("webhook", 300);
  if (body.payment)
    await new Billing().webhook(
      body.payment.id,
      body.payment.externalReference,
      body.id,
    );
  return response({ ok: true });
}
export async function admin(r: Request) {
  const u = await administrator();
  const s = new Store();
  await s.rate("admin:" + u.id, 60);
  const query = new URL(r.url).searchParams;
  const parsed = z
    .object({
      offset: z.coerce.number().int().min(0).max(1000000),
      limit: z.coerce.number().int().min(1).max(100),
      refundOffset: z.coerce.number().int().min(0).max(1000000),
      refundLimit: z.coerce.number().int().min(1).max(100),
    })
    .safeParse({
      offset: query.get("offset") ?? 0,
      limit: query.get("limit") ?? 50,
      refundOffset: query.get("refundOffset") ?? 0,
      refundLimit: query.get("refundLimit") ?? 50,
    });
  if (!parsed.success) fail(400, "INVALID_PAGE", "Página inválida.");
  const p = parsed.data;
  const [orders, refunds] = await Promise.all([
    s.orderPage({ offset: p.offset, limit: p.limit, refundOnly: false }),
    s.orderPage({
      offset: p.refundOffset,
      limit: p.refundLimit,
      refundOnly: true,
    }),
  ]);
  return response({
    orders: orders.orders.map((o) => ({ ...summary(o), userId: o.user_id })),
    refundRequests: refunds.orders.map((o) => ({
      id: o.id,
      orderId: o.id,
      userId: o.user_id,
      status: o.refund_state,
      createdAt: o.created_at,
    })),
    orderPagination: {
      offset: p.offset,
      limit: p.limit,
      total: orders.total,
      hasMore: p.offset + orders.orders.length < orders.total,
    },
    refundPagination: {
      offset: p.refundOffset,
      limit: p.refundLimit,
      total: refunds.total,
      hasMore: p.refundOffset + refunds.orders.length < refunds.total,
    },
    serviceAvailable: getConfig().serviceAvailable,
  });
}
export async function adminRefund(r: Request) {
  sameOrigin(r);
  const u = await administrator();
  await new Store().rate("admin-refund:" + u.id, 10);
  const body = await jsonBody(
    r,
    z.object({ orderId: uuid, confirm: z.literal(true) }).strict(),
  );
  return response(summary(await new Billing().refund(body.orderId, u.id)));
}
export async function connect(r: Request) {
  const u = await authenticatedUser(),
    c = getConfig(),
    url = new URL(r.url);
  if (!c.remoteOrigin || !c.mcpSecret || c.mcpSecret.length < 32)
    fail(503, "CONFIGURATION", "Conexão ainda não liberada.");
  const parsed = z
    .object({
      state: z
        .string()
        .min(16)
        .max(256)
        .regex(/^[a-zA-Z0-9_-]+$/),
      redirect_uri: z.literal(c.remoteOrigin + "/auth/commercial/callback"),
    })
    .safeParse({
      state: url.searchParams.get("state"),
      redirect_uri: url.searchParams.get("redirect_uri"),
    });
  if (!parsed.success)
    fail(400, "CALLBACK_INVALID", "Solicitação de conexão inválida.");
  const s = new Store();
  await s.rate("connect:" + u.id, 10);
  const ticket = randomBytes(32).toString("base64url");
  await s.rpc("portal_ticket_create", {
    p_hash: hash(ticket),
    p_user: u.id,
    p_email: u.email,
    p_state: parsed.data.state,
    p_audience: c.remoteOrigin,
  });
  const target = new URL(parsed.data.redirect_uri);
  target.searchParams.set("ticket", ticket);
  target.searchParams.set("state", parsed.data.state);
  return response({ redirectUrl: target.href });
}
export async function exchange(r: Request) {
  const c = mcpAuth(r);
  const body = await jsonBody(
    r,
    z
      .object({
        ticket: z
          .string()
          .min(40)
          .max(64)
          .regex(/^[a-zA-Z0-9_-]+$/),
        state: z.string().min(16).max(256),
        redirect_uri: z.literal(c.remoteOrigin + "/auth/commercial/callback"),
      })
      .strict(),
  );
  const s = new Store();
  await s.rate("exchange", 120);
  const result = await s.rpc("portal_ticket_exchange", {
    p_hash: hash(body.ticket),
    p_state: body.state,
    p_audience: c.remoteOrigin,
  });
  if (!result)
    fail(401, "TICKET_INVALID", "Ticket inválido, expirado ou já utilizado.");
  return response(result);
}
export async function authorize(r: Request) {
  const c = mcpAuth(r);
  const body = await jsonBody(
    r,
    z
      .object({
        subject: uuid,
        operation_id: uuid,
        mode: z.enum(["inspect", "execute"]),
      })
      .strict(),
  );
  const s = new Store();
  await s.rate("authorize:" + body.subject, 300);
  return response(
    await new Billing(s).authorize(body.subject, body.operation_id, body.mode, c.serviceAvailable),
  );
}
export async function callback(r: Request) {
  const url = new URL(r.url),
    c = getConfig();
  if (!c.portalOrigin) fail(503, "CONFIGURATION", "Portal não configurado.");
  const code = url.searchParams.get("code");
  const token = url.searchParams.get("token_hash"),
    type = url.searchParams.get("type");
  const failure = () => Response.redirect(c.portalOrigin + "/login?error=confirmation", 303);
  if ((code && token) || (!code && !(token && ["email", "signup"].includes(type ?? "")))) return failure();
  const client = await sessionClient();
  let error: unknown;
  if (code) ({ error } = await client.auth.exchangeCodeForSession(code));
  else if (
    token &&
    (type === "email" || type === "signup")
  )
    ({ error } = await client.auth.verifyOtp({ token_hash: token, type }));
  else error = true;
  if (error) return failure();
  const { data, error: identityError } = await client.auth.getUser();
  if (identityError || !data.user?.email || !data.user.email_confirmed_at) {
    await client.auth.signOut({ scope: "local" });
    return failure();
  }
  const target = new URL(safeNext(url.searchParams.get("next")), c.portalOrigin);
  return Response.redirect(
    target.origin === c.portalOrigin
      ? target
      : new URL("/dashboard", c.portalOrigin),
    303,
  );
}
