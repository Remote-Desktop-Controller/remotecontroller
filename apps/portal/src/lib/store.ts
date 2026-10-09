import "server-only";
import { serviceClient } from "./supabase";
import { fail } from "./http";
export type Entitlement = {
  activated_at: string | null;
  expires_at: string | null;
  revoked_at: string | null;
};
export type Order = {
  id: string;
  user_id: string;
  amount_cents: number;
  status: string;
  creation_state: string;
  customer_id: string | null;
  payment_id: string | null;
  payment_url: string | null;
  refund_state: string | null;
  created_at: string;
  entitlement?: Entitlement | null;
};
type RawOrder = Omit<Order, "entitlement"> & {
  entitlement?: Entitlement | Entitlement[] | null;
};
function normalize(row: RawOrder): Order {
  return {
    ...row,
    entitlement: Array.isArray(row.entitlement)
      ? (row.entitlement[0] ?? null)
      : (row.entitlement ?? null),
  };
}
const orderFields =
  "*,entitlement:portal_entitlements(activated_at,expires_at,revoked_at)";
export class Store {
  readonly db = serviceClient();
  async rpc<T>(name: string, args: Record<string, unknown>): Promise<T> {
    const { data, error } = await this.db.rpc(name, args);
    if (error) fail(503, "DATABASE", "Não foi possível atualizar o estado.");
    return data as T;
  }
  async rate(key: string, limit = 10, seconds = 60) {
    if (
      !(await this.rpc<boolean>("portal_rate_limit", {
        p_key: key,
        p_limit: limit,
        p_seconds: seconds,
      }))
    )
      fail(429, "RATE_LIMIT", "Aguarde antes de tentar novamente.");
  }
  async findOrder(id: string): Promise<Order | null> {
    const { data, error } = await this.db
      .from("portal_orders")
      .select(orderFields)
      .eq("id", id)
      .maybeSingle();
    if (error) fail(503, "DATABASE", "Consulta indisponível.");
    return data ? normalize(data as unknown as RawOrder) : null;
  }
  async order(id: string, user?: string): Promise<Order> {
    const data = await this.findOrder(id);
    if (!data || (user && data.user_id !== user))
      fail(403, "ORDER_ACCESS", "Pedido não disponível para esta conta.");
    return data;
  }
  async orders(user?: string) {
    return (await this.orderPage({ user, offset: 0, limit: 100 })).orders;
  }
  async authorizationOrders(user: string): Promise<Order[]> {
    // Filter before bounding: old/expired history must not hide a live credit.
    const [live, open] = await Promise.all([
      this.db.from("portal_orders")
        .select("*,entitlement:portal_entitlements!inner(activated_at,expires_at,revoked_at)")
        .eq("user_id", user)
        .is("entitlement.revoked_at", null)
        .or(`activated_at.is.null,expires_at.gt.${new Date().toISOString()}`, { referencedTable: "entitlement" })
        .order("created_at").order("id").limit(17),
      this.db.from("portal_orders").select(orderFields)
        .eq("user_id", user).eq("status", "PENDING_PAYMENT")
        .neq("creation_state", "RESERVED")
        .order("created_at").order("id").limit(17),
    ]);
    if (live.error || open.error) fail(503, "DATABASE", "Conciliação financeira indisponível.");
    const orders = new Map<string, Order>();
    for (const row of [...(live.data ?? []), ...(open.data ?? [])] as unknown as RawOrder[]) {
      const order = normalize(row);
      orders.set(order.id, order);
    }
    if (orders.size > 16) fail(503, "RECONCILIATION_REQUIRED", "Pedidos requerem conciliação administrativa.");
    return [...orders.values()];
  }
  async orderPage(options: {
    user?: string;
    refundOnly?: boolean;
    offset: number;
    limit: number;
  }) {
    let q = this.db
      .from("portal_orders")
      .select(orderFields, { count: "exact" })
      .order("created_at", { ascending: false })
      .order("id", { ascending: false })
      .range(options.offset, options.offset + options.limit - 1);
    if (options.user) q = q.eq("user_id", options.user);
    if (options.refundOnly) q = q.not("refund_state", "is", null);
    const { data, error, count } = await q;
    if (error) fail(503, "DATABASE", "Consulta indisponível.");
    return {
      orders: ((data ?? []) as unknown as RawOrder[]).map(normalize),
      total: count ?? 0,
    };
  }
  async claimOrder(
    id: string,
    field = "creation_state",
    from = "RESERVED",
    to = "CREATING",
  ) {
    let query = this.db
      .from("portal_orders")
      .update({ [field]: to })
      .eq("id", id)
      .eq(field, from);
    if (field === "creation_state")
      query = query.eq("status", "PENDING_PAYMENT");
    const { data, error } = await query.select("id");
    if (error) fail(503, "DATABASE", "Não foi possível reservar a operação.");
    return !!data?.length;
  }
  async patchOrder(id: string, fields: Record<string, unknown>) {
    const { error } = await this.db
      .from("portal_orders")
      .update({ ...fields, updated_at: new Date().toISOString() })
      .eq("id", id);
    if (error) fail(503, "DATABASE", "Não foi possível atualizar o pedido.");
  }
  async cancelReserved(id: string) {
    const { data, error } = await this.db
      .from("portal_orders")
      .update({ status: "CANCELED", updated_at: new Date().toISOString() })
      .eq("id", id)
      .eq("status", "PENDING_PAYMENT")
      .eq("creation_state", "RESERVED")
      .is("payment_id", null)
      .select("id");
    if (error) fail(503, "DATABASE", "Cancelamento indisponível.");
    return !!data?.length;
  }
  async requestRefundReservation(id: string) {
    const { error } = await this.db
      .from("portal_orders")
      .update({
        refund_state: "REQUESTED",
        updated_at: new Date().toISOString(),
      })
      .eq("id", id)
      .is("refund_state", null)
      .in("status", [
        "PAID_AWAITING_SERVICE",
        "READY_TO_ACTIVATE",
        "ACTIVE",
        "EXPIRED",
      ]);
    if (error) fail(503, "DATABASE", "Não foi possível solicitar o estorno.");
  }
  async markRefundUnknown(id: string) {
    const { error } = await this.db
      .from("portal_orders")
      .update({ refund_state: "UNKNOWN", updated_at: new Date().toISOString() })
      .eq("id", id)
      .eq("refund_state", "CREATING");
    if (error) fail(503, "DATABASE", "Estorno requer conciliação.");
  }
  async releaseOrder(id: string) {
    const { error } = await this.db
      .from("portal_orders")
      .update({ creation_state: "RESERVED" })
      .eq("id", id)
      .eq("creation_state", "CREATING")
      .is("payment_id", null);
    if (error) fail(503, "DATABASE", "Pedido requer conciliação.");
  }
  async releaseRefund(id: string) {
    const { error } = await this.db
      .from("portal_orders")
      .update({ refund_state: "REQUESTED" })
      .eq("id", id)
      .eq("refund_state", "CREATING");
    if (error) fail(503, "DATABASE", "Estorno requer conciliação.");
  }
  async releaseCustomer(user: string) {
    const { error } = await this.db
      .from("portal_customers")
      .update({ creation_state: "RESERVED" })
      .eq("user_id", user)
      .eq("creation_state", "CREATING")
      .is("provider_id", null);
    if (error) fail(503, "DATABASE", "Cliente requer conciliação.");
  }
  async customer(user: string) {
    const { data, error } = await this.db
      .from("portal_customers")
      .select("*")
      .eq("user_id", user)
      .single();
    if (error) fail(503, "DATABASE", "Cliente indisponível.");
    return data as {
      user_id: string;
      provider_id: string | null;
      creation_state: string;
    };
  }
  async claimCustomer(user: string) {
    const { data, error } = await this.db
      .from("portal_customers")
      .update({ creation_state: "CREATING" })
      .eq("user_id", user)
      .eq("creation_state", "RESERVED")
      .select("user_id");
    if (error) fail(503, "DATABASE", "Não foi possível reservar o cliente.");
    return !!data?.length;
  }
  async patchCustomer(user: string, fields: Record<string, unknown>) {
    const { error } = await this.db
      .from("portal_customers")
      .update(fields)
      .eq("user_id", user);
    if (error) fail(503, "DATABASE", "Não foi possível atualizar o cliente.");
  }
  async audit(user: string, order: string, key: string, kind: string) {
    const { error } = await this.db
      .from("portal_events")
      .upsert(
        { user_id: user, order_id: order, event_key: key, kind },
        { onConflict: "event_key", ignoreDuplicates: true },
      );
    if (error) fail(503, "DATABASE", "Não foi possível registrar o evento.");
  }
}
