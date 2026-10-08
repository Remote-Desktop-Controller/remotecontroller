import "server-only";
import type { Config } from "./config";
import { fail, HttpError } from "./http";
export class ProviderRejection extends HttpError {
  constructor() {
    super(
      400,
      "PROVIDER_REJECTED",
      "A operadora recusou os dados ou a operação. Confira as informações antes de tentar novamente.",
    );
  }
}
export type Payment = {
  id: string;
  customer: string;
  externalReference: string;
  value: number;
  billingType: string;
  status: string;
  invoiceUrl?: string;
  deleted?: boolean;
  refunds?: { status: string; value: number }[];
};
type Customer = { id: string; externalReference: string };
export function validatePayment(
  order: {
    id: string;
    customer_id: string | null;
    amount_cents: number;
    payment_id: string | null;
  },
  p: Payment,
) {
  if (
    !p ||
    p.id !== order.payment_id ||
    p.customer !== order.customer_id ||
    p.externalReference !== order.id ||
    !Number.isFinite(p.value) ||
    Math.round(p.value * 100) !== Number(order.amount_cents) ||
    Math.abs(p.value * 100 - Math.round(p.value * 100)) > 0.000001 ||
    p.billingType !== "PIX"
  )
    fail(409, "PAYMENT_MISMATCH", "Dados da cobrança divergentes.");
}
export function paymentUrl(value: string | undefined) {
  try {
    const u = new URL(value ?? "");
    if (
      u.protocol === "https:" &&
      !u.username &&
      !u.password &&
      (u.hostname === "asaas.com" || u.hostname.endsWith(".asaas.com"))
    )
      return u.href;
  } catch {}
  fail(503, "PROVIDER_RESPONSE", "Fatura indisponível.");
}
export class Asaas {
  readonly base: string;
  constructor(
    readonly config: Config,
    readonly transport: typeof fetch = fetch,
  ) {
    this.base = config.sandbox
      ? "https://api-sandbox.asaas.com/v3"
      : "https://api.asaas.com/v3";
  }
  async request<T>(path: string, method = "GET", body?: unknown): Promise<T> {
    if (!this.config.asaasApiKey)
      fail(503, "CONFIGURATION", "Pagamento ainda não configurado.");
    let res: Response;
    try {
      res = await this.transport(this.base + path, {
        method,
        headers: {
          access_token: this.config.asaasApiKey,
          "Content-Type": "application/json",
          "User-Agent": "RemoteController-Portal/0.1",
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(15000),
        cache: "no-store",
        redirect: "error",
      });
    } catch {
      fail(
        method === "GET" ? 503 : 409,
        "PROVIDER_UNKNOWN",
        "Resultado ainda desconhecido. Consulte o pedido; não repita a cobrança.",
      );
    }
    if (!res.ok) {
      if (method !== "GET" && [400, 422].includes(res.status)) {
        let rejected = false;
        try {
          const error = (await res.json()) as { errors?: unknown };
          rejected =
            Array.isArray(error.errors) &&
            error.errors.length > 0 &&
            error.errors.every(
              (item) =>
                item &&
                typeof item === "object" &&
                /^(invalid_|required_|missing_|insufficient_)/.test(
                  String((item as { code?: unknown }).code),
                ),
            );
        } catch {}
        if (rejected) throw new ProviderRejection();
      }
      fail(
        method === "GET" ? 503 : 409,
        "PROVIDER_UNAVAILABLE",
        "Não foi possível confirmar a operação com o provedor.",
      );
    }
    try {
      return (await res.json()) as T;
    } catch {
      fail(
        method === "GET" ? 503 : 409,
        "PROVIDER_UNKNOWN",
        "Resposta ainda não confirmada.",
      );
    }
  }
  createCustomer(user: string, name: string, cpfCnpj: string, email: string) {
    return this.request<Customer>("/customers", "POST", {
      name,
      cpfCnpj,
      email,
      externalReference: user,
      notificationDisabled: true,
    });
  }
  async findCustomer(user: string) {
    const r = await this.request<{ data: Customer[]; hasMore: boolean }>(
      "/customers?externalReference=" + encodeURIComponent(user) + "&limit=100",
    );
    if (r.hasMore || r.data.length > 1)
      fail(
        409,
        "PROVIDER_DUPLICATE",
        "Cliente requer conciliação administrativa.",
      );
    return r.data[0] ?? null;
  }
  createPayment(customer: string, order: string, cents: number) {
    if (!this.config.portalOrigin)
      fail(503, "CONFIGURATION", "Retorno do pagamento não configurado.");
    const due = new Date(Date.now() + 3 * 86400000).toISOString().slice(0, 10);
    return this.request<Payment>("/payments", "POST", {
      customer,
      billingType: "PIX",
      value: cents / 100,
      dueDate: due,
      externalReference: order,
      description:
        "Remote Controller: 30 dias pré-pagos. Prazo inicia no primeiro uso autorizado.",
      fine: { value: 0 },
      interest: { value: 0 },
      callback: {
        successUrl:
          this.config.portalOrigin +
          "/purchase?orderId=" +
          encodeURIComponent(order),
        autoRedirect: true,
      },
    });
  }
  async findPayment(order: string) {
    const r = await this.request<{ data: Payment[]; hasMore: boolean }>(
      "/payments?externalReference=" + encodeURIComponent(order) + "&limit=100",
    );
    if (r.hasMore || r.data.length > 1)
      fail(
        409,
        "PROVIDER_DUPLICATE",
        "Cobrança requer conciliação administrativa.",
      );
    return r.data[0] ?? null;
  }
  payment(id: string) {
    return this.request<Payment>("/payments/" + encodeURIComponent(id));
  }
  cancel(id: string) {
    return this.request("/payments/" + encodeURIComponent(id), "DELETE");
  }
  refund(id: string) {
    return this.request(
      "/payments/" + encodeURIComponent(id) + "/refund",
      "POST",
      {},
    );
  }
}
