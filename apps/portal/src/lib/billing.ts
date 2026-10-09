import "server-only";
import {
  Asaas,
  ProviderRejection,
  paymentUrl,
  validatePayment,
  type Payment,
} from "./asaas";
import { Store, type Order } from "./store";
import { getConfig } from "./config";
import { fail, uuid } from "./http";
export class Billing {
  constructor(
    readonly store: Store = new Store(),
    readonly provider: Asaas = new Asaas(getConfig()),
  ) {}
  async authorize(
    subject: string,
    operationId: string,
    mode: "inspect" | "execute",
    available: boolean,
  ) {
    const verifiedOrders: string[] = [];
    if (available) {
      const orders = await this.store.authorizationOrders(subject);
      // Bound both provider load and latency; every call is GET and outside SQL locks.
      if (orders.length > 16)
        fail(503, "RECONCILIATION_REQUIRED", "Pedidos requerem conciliação administrativa.");
      await Promise.all(
        orders.map(async (initial) => {
          if (initial.user_id !== subject)
            fail(503, "PAYMENT_UNVERIFIED", "Conciliação financeira não confirmada.");
          const { order, p } = await this.source(initial);
          await this.apply(order, p);
          const refunded =
            p.status === "REFUNDED" ||
            p.status === "PARTIALLY_REFUNDED" ||
            p.refunds?.some((r) => r.status === "DONE" && r.value > 0);
          if (
            !refunded &&
            initial.status !== "PENDING_PAYMENT" &&
            (p.status !== "RECEIVED" || p.deleted)
          )
            fail(503, "PAYMENT_UNVERIFIED", "Pagamento ainda não confirmado pela operadora.");
          if (!refunded && p.status === "RECEIVED" && !p.deleted)
            verifiedOrders.push(order.id);
        }),
      );
    }
    return this.store.rpc("portal_authorize", {
      p_user: subject,
      p_operation: operationId,
      p_mode: mode,
      p_available: available,
      p_verified_orders: verifiedOrders,
    });
  }
  async customer(
    user: { id: string; email: string },
    name: string,
    cpfCnpj: string,
  ) {
    const c = await this.store.customer(user.id);
    if (c.provider_id) return c.provider_id;
    const claimed = await this.store.claimCustomer(user.id);
    if (!claimed) {
      const found = await this.provider.findCustomer(user.id);
      if (!found || found.externalReference !== user.id)
        fail(409, "CUSTOMER_UNKNOWN", "Cadastro de cobrança em conciliação.");
      await this.store.patchCustomer(user.id, {
        provider_id: found.id,
        creation_state: "READY",
      });
      return found.id;
    }
    try {
      const created = await this.provider.createCustomer(
        user.id,
        name,
        cpfCnpj,
        user.email,
      );
      if (!created.id || created.externalReference !== user.id)
        fail(409, "CUSTOMER_UNKNOWN", "Cadastro em conciliação.");
      await this.store.patchCustomer(user.id, {
        provider_id: created.id,
        creation_state: "READY",
      });
      return created.id;
    } catch (e) {
      if (e instanceof ProviderRejection)
        await this.store.releaseCustomer(user.id);
      else
        await this.store.patchCustomer(user.id, { creation_state: "UNKNOWN" });
      throw e;
    }
  }
  async checkout(
    user: { id: string; email: string },
    name: string,
    cpfCnpj: string,
  ) {
    const c = getConfig();
    if (!c.salesEnabled || !c.priceCents)
      fail(503, "SALES_DISABLED", "Vendas ainda não disponíveis.");
    if (!this.provider.config.asaasApiKey || !this.provider.config.portalOrigin)
      fail(503, "CONFIGURATION", "Pagamento ainda não configurado.");
    let order = await this.store.rpc<Order>("portal_checkout_reserve", {
      p_user: user.id,
      p_amount: c.priceCents,
    });
    if (order.payment_id) {
      order = await this.sync(order.id, user.id);
      return { orderId: order.id, paymentUrl: order.payment_url };
    }
    const customer = await this.customer(user, name, cpfCnpj);
    if (!(await this.store.claimOrder(order.id))) {
      order = await this.sync(order.id, user.id);
      return { orderId: order.id, paymentUrl: order.payment_url };
    }
    let submitted = false;
    try {
      await this.store.patchOrder(order.id, { customer_id: customer });
      submitted = true;
      const p = await this.provider.createPayment(
        customer,
        order.id,
        Number(order.amount_cents),
      );
      validatePayment({ ...order, customer_id: customer, payment_id: p.id }, p);
      const url = paymentUrl(p.invoiceUrl);
      await this.store.rpc("portal_checkout_complete", {
        p_order: order.id,
        p_customer: customer,
        p_payment: p.id,
        p_url: url,
      });
      await this.apply(
        { ...order, customer_id: customer, payment_id: p.id },
        p,
      );
      return { orderId: order.id, paymentUrl: url };
    } catch (e) {
      if (!submitted || e instanceof ProviderRejection)
        await this.store.releaseOrder(order.id);
      else await this.store.patchOrder(order.id, { creation_state: "UNKNOWN" });
      throw e;
    }
  }
  async source(order: Order) {
    let p: Payment;
    if (order.payment_id) p = await this.provider.payment(order.payment_id);
    else {
      const found = await this.provider.findPayment(order.id);
      if (!found)
        fail(
          409,
          "PAYMENT_UNKNOWN",
          "Cobrança em conciliação. Não repita o pagamento.",
        );
      p = found;
      if (!order.customer_id) {
        const c = await this.store.customer(order.user_id);
        if (!c.provider_id)
          fail(409, "CUSTOMER_UNKNOWN", "Cliente em conciliação.");
        order = { ...order, customer_id: c.provider_id };
      }
      validatePayment({ ...order, payment_id: p.id }, p);
      await this.store.rpc("portal_checkout_complete", {
        p_order: order.id,
        p_customer: order.customer_id,
        p_payment: p.id,
        p_url: paymentUrl(p.invoiceUrl),
      });
      order = { ...order, payment_id: p.id };
    }
    validatePayment(order, p);
    return { order, p };
  }
  async apply(order: Order, p: Payment, event?: string) {
    validatePayment(order, p);
    const refunded =
      p.status === "PARTIALLY_REFUNDED" ||
      p.refunds?.some((r) => r.status === "DONE" && r.value > 0);
    const status =
      p.status === "REFUNDED"
        ? "REFUNDED"
        : refunded
          ? "REFUND_PARTIALLY_DONE"
          : p.status === "RECEIVED"
            ? "RECEIVED"
            : p.deleted
              ? "DELETED"
              : p.status;
    return this.store.rpc<Order>("portal_payment_apply", {
      p_order: order.id,
      p_event: event ?? `sync:${order.id}:${status}`,
      p_status: status,
    });
  }
  async sync(id: string, user?: string, event?: string) {
    const initial = await this.store.order(id, user);
    const { order, p } = await this.source(initial);
    await this.apply(order, p, event);
    return this.store.order(id, user);
  }
  async webhook(paymentId: string, reference: unknown, event: string) {
    const p = await this.provider.payment(paymentId);
    if (p.id !== paymentId)
      fail(409, "PAYMENT_MISMATCH", "Cobrança divergente.");
    const trusted = uuid.safeParse(p.externalReference);
    if (!trusted.success) return null;
    let order = await this.store.findOrder(trusted.data);
    if (!order) return null;
    if (
      reference !== undefined &&
      reference !== null &&
      reference !== p.externalReference
    )
      fail(409, "PAYMENT_MISMATCH", "Referência da cobrança divergente.");
    if (!order.customer_id) {
      const c = await this.store.customer(order.user_id);
      if (!c.provider_id)
        fail(409, "CUSTOMER_UNKNOWN", "Cliente em conciliação.");
      order = { ...order, customer_id: c.provider_id };
    }
    validatePayment({ ...order, payment_id: order.payment_id ?? p.id }, p);
    if (!order.payment_id) {
      await this.store.rpc("portal_checkout_complete", {
        p_order: order.id,
        p_customer: order.customer_id,
        p_payment: p.id,
        p_url: paymentUrl(p.invoiceUrl),
      });
      order = { ...order, payment_id: p.id };
    }
    return this.apply(order, p, "webhook:" + event);
  }
  async cancel(id: string, user: string) {
    let order = await this.store.order(id, user);
    if (order.status === "CANCELED") return order;
    if (
      !order.payment_id &&
      order.creation_state === "RESERVED" &&
      (await this.store.cancelReserved(id))
    ) {
      await this.store.audit(user, id, "cancel:" + id, "CANCELED");
      return this.store.order(id, user);
    }
    order = await this.sync(id, user);
    if (order.status === "CANCELED") return order;
    if (order.status !== "PENDING_PAYMENT")
      fail(409, "REFUND_REQUIRED", "Pagamento recebido. Solicite reembolso.");
    // DELETE only cancels pending payment; fresh GET after DELETE catches a concurrent settlement.
    try {
      await this.provider.cancel(order.payment_id!);
    } catch {
      order = await this.sync(id, user);
      if (order.status !== "PENDING_PAYMENT") return order;
      fail(409, "CANCEL_UNKNOWN", "Cancelamento em conciliação.");
    }
    return this.sync(id, user);
  }
  async requestRefund(id: string, user: string) {
    const order = await this.store.order(id, user);
    if (
      ![
        "PAID_AWAITING_SERVICE",
        "READY_TO_ACTIVATE",
        "ACTIVE",
        "EXPIRED",
      ].includes(order.status)
    )
      fail(
        409,
        "REFUND_UNAVAILABLE",
        "Este pedido não permite solicitar reembolso.",
      );
    await this.store.requestRefundReservation(id);
    await this.store.audit(
      user,
      id,
      "refund-request:" + id,
      "REFUND_REQUESTED",
    );
    return {
      ok: true,
      message:
        "Solicitação registrada. O acesso só muda após confirmação do estorno.",
    };
  }
  async refund(id: string, admin: string) {
    const order = await this.sync(id);
    if (order.status === "REFUNDED") return order;
    if (
      ![
        "PAID_AWAITING_SERVICE",
        "READY_TO_ACTIVATE",
        "ACTIVE",
        "EXPIRED",
      ].includes(order.status)
    )
      fail(409, "REFUND_UNAVAILABLE", "Pedido sem pagamento recebido.");
    await this.store.requestRefundReservation(id);
    if (
      !(await this.store.claimOrder(
        id,
        "refund_state",
        "REQUESTED",
        "CREATING",
      ))
    )
      fail(
        409,
        "REFUND_UNKNOWN",
        "Estorno em conciliação; não envie novamente.",
      );
    try {
      await this.store.audit(
        order.user_id,
        id,
        "refund-admin:" + id,
        "REFUND_APPROVED:" + admin,
      );
    } catch (e) {
      await this.store.releaseRefund(id);
      throw e;
    }
    try {
      await this.provider.refund(order.payment_id!);
    } catch (e) {
      if (e instanceof ProviderRejection) await this.store.releaseRefund(id);
      else await this.store.markRefundUnknown(id);
      throw e;
    }
    await this.store.markRefundUnknown(id);
    return this.sync(id);
  }
}
