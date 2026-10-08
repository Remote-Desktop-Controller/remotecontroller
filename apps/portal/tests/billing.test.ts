import { describe, it, expect, vi, afterEach } from "vitest";
import { Billing } from "../src/lib/billing";
import { Asaas } from "../src/lib/asaas";
import { getConfig } from "../src/lib/config";
import type { Order, Store } from "../src/lib/store";
afterEach(() => vi.unstubAllEnvs());
function fixture() {
  const order: Order = {
    id: "order",
    user_id: "user",
    amount_cents: 2900,
    status: "PENDING_PAYMENT",
    creation_state: "UNKNOWN",
    customer_id: "customer",
    payment_id: null,
    payment_url: null,
    refund_state: null,
    created_at: new Date().toISOString(),
  };
  const calls: string[] = [];
  const store = {
    order: async () => ({ ...order }),
    rpc: async (name: string, args: Record<string, unknown>) => {
      calls.push(name);
      if (name === "portal_checkout_reserve") return { ...order };
      if (name === "portal_checkout_complete") {
        order.payment_id = String(args.p_payment);
        order.payment_url = String(args.p_url);
        order.creation_state = "READY";
      }
      if (name === "portal_payment_apply" && args.p_status === "RECEIVED")
        order.status = "PAID_AWAITING_SERVICE";
      return { ...order };
    },
    customer: async () => ({
      user_id: "user",
      provider_id: "customer",
      creation_state: "READY",
    }),
    claimCustomer: async () => false,
    patchCustomer: async () => {},
    claimOrder: async () => false,
    patchOrder: async (_id: string, fields: Record<string, unknown>) => {
      Object.assign(order, fields);
    },
    requestRefundReservation: async () => {
      if (order.refund_state === null) order.refund_state = "REQUESTED";
    },
    audit: async () => {},
  };
  const payment = {
    id: "payment",
    customer: "customer",
    externalReference: "order",
    value: 29,
    billingType: "PIX",
    status: "RECEIVED",
    invoiceUrl: "https://sandbox.asaas.com/i/test",
  };
  let posts = 0;
  const client = new Asaas(
    { ...getConfig(), asaasApiKey: "fixture" },
    async (input, init) => {
      if (init?.method === "POST") posts++;
      return Response.json(
        String(input).includes("externalReference")
          ? { data: [payment], hasMore: false }
          : payment,
      );
    },
  );
  return {
    store,
    order,
    calls,
    client,
    get posts() {
      return posts;
    },
  };
}
describe("billing HTTP recovery", () => {
  it("reconciles an ambiguous checkout with GET and never posts a duplicate", async () => {
    const f = fixture();
    const result = await new Billing(
      f.store as unknown as Store,
      f.client,
    ).sync("order", "user");
    expect(result.status).toBe("PAID_AWAITING_SERVICE");
    expect(f.posts).toBe(0);
    expect(f.calls).toContain("portal_checkout_complete");
  });
  it("unknown missing payment does not POST or reset reservation", async () => {
    const f = fixture();
    const client = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async () => Response.json({ data: [], hasMore: false }),
    );
    await expect(
      new Billing(f.store as unknown as Store, client).sync("order", "user"),
    ).rejects.toMatchObject({ status: 409 });
    expect(f.order.creation_state).toBe("UNKNOWN");
  });
  it("foreign source payments cannot credit the ledger", async () => {
    const f = fixture();
    const client = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async () =>
        Response.json({
          data: [
            {
              id: "foreign",
              customer: "other",
              externalReference: "order",
              value: 29,
              billingType: "PIX",
              status: "RECEIVED",
              invoiceUrl: "https://sandbox.asaas.com/i/test",
            },
          ],
        }),
    );
    await expect(
      new Billing(f.store as unknown as Store, client).sync("order", "user"),
    ).rejects.toMatchObject({ status: 409 });
    expect(f.calls).not.toContain("portal_payment_apply");
  });
  it("refund request alone leaves paid entitlement intact", async () => {
    const f = fixture();
    f.order.status = "PAID_AWAITING_SERVICE";
    await new Billing(f.store as unknown as Store, f.client).requestRefund(
      "order",
      "user",
    );
    expect(f.order.refund_state).toBe("REQUESTED");
    expect(f.calls).not.toContain("portal_payment_apply");
  });
  it("cancels an unclaimed payment reservation without provider I/O", async () => {
    const f = fixture();
    f.order.creation_state = "RESERVED";
    const store = {
      ...f.store,
      cancelReserved: async () => {
        f.order.status = "CANCELED";
        return true;
      },
    };
    const client = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async () => {
        throw new Error("provider must not be called");
      },
    );
    expect(
      (
        await new Billing(store as unknown as Store, client).cancel(
          "order",
          "user",
        )
      ).status,
    ).toBe("CANCELED");
  });
  it("a stale user refund request cannot reset an administrator in-flight reservation", async () => {
    const f = fixture();
    f.order.status = "PAID_AWAITING_SERVICE";
    f.order.refund_state = "CREATING";
    const store = {
      ...f.store,
      order: async () => ({ ...f.order, refund_state: null }),
      requestRefundReservation: async () => {
        if (f.order.refund_state === null) f.order.refund_state = "REQUESTED";
      },
    };
    await new Billing(store as unknown as Store, f.client).requestRefund(
      "order",
      "user",
    );
    expect(f.order.refund_state).toBe("CREATING");
  });
  it("definitive customer validation rejection releases reservation for corrected data", async () => {
    const f = fixture();
    let state = "RESERVED";
    const store = {
      ...f.store,
      customer: async () => ({ provider_id: null, creation_state: state }),
      claimCustomer: async () => {
        state = "CREATING";
        return true;
      },
      patchCustomer: async (_id: string, fields: Record<string, unknown>) => {
        state = String(fields.creation_state);
      },
      releaseCustomer: async () => {
        state = "RESERVED";
      },
    };
    const provider = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async () =>
        Response.json(
          { errors: [{ code: "invalid_cpfCnpj", description: "Invalid" }] },
          { status: 400 },
        ),
    );
    await expect(
      new Billing(store as unknown as Store, provider).customer(
        { id: "user", email: "user@example.com" },
        "Maria",
        "12345678901",
      ),
    ).rejects.toMatchObject({ code: "PROVIDER_REJECTED" });
    expect(state).toBe("RESERVED");
  });
  it("definitive payment validation rejection releases payment reservation", async () => {
    vi.stubEnv("SALES_ENABLED", "true");
    vi.stubEnv("PORTAL_PRICE_CENTS", "2900");
    const f = fixture();
    f.order.creation_state = "RESERVED";
    const store = {
      ...f.store,
      claimOrder: async () => {
        f.order.creation_state = "CREATING";
        return true;
      },
      releaseOrder: async () => {
        f.order.creation_state = "RESERVED";
      },
    };
    const provider = new Asaas(
      {
        ...getConfig(),
        asaasApiKey: "fixture",
        portalOrigin: "https://portal.example",
      },
      async () =>
        Response.json(
          { errors: [{ code: "invalid_value", description: "Invalid" }] },
          { status: 422 },
        ),
    );
    await expect(
      new Billing(store as unknown as Store, provider).checkout(
        { id: "user", email: "user@example.com" },
        "Maria",
        "12345678901",
      ),
    ).rejects.toMatchObject({ code: "PROVIDER_REJECTED" });
    expect(f.order.creation_state).toBe("RESERVED");
  });
  it("refund audit failure before HTTP releases the admin claim without posting", async () => {
    const f = fixture();
    f.order.status = "PAID_AWAITING_SERVICE";
    f.order.payment_id = "payment";
    f.order.refund_state = "REQUESTED";
    const store = {
      ...f.store,
      claimOrder: async () => {
        f.order.refund_state = "CREATING";
        return true;
      },
      audit: async () => {
        throw new Error("database unavailable");
      },
      releaseRefund: async () => {
        f.order.refund_state = "REQUESTED";
      },
    };
    await expect(
      new Billing(store as unknown as Store, f.client).refund("order", "admin"),
    ).rejects.toThrow("database unavailable");
    expect(f.posts).toBe(0);
    expect(f.order.refund_state).toBe("REQUESTED");
  });
  it("ambiguous customer provider failure stays UNKNOWN and retry only reconciles GET", async () => {
    const f = fixture();
    let state = "RESERVED",
      posts = 0;
    const store = {
      ...f.store,
      customer: async () => ({ provider_id: null, creation_state: state }),
      claimCustomer: async () => {
        if (state !== "RESERVED") return false;
        state = "CREATING";
        return true;
      },
      patchCustomer: async (_id: string, fields: Record<string, unknown>) => {
        state = String(fields.creation_state);
      },
    };
    const provider = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async (_input, init) => {
        if (init?.method === "POST") {
          posts++;
          return new Response("", { status: 503 });
        }
        return Response.json({ data: [], hasMore: false });
      },
    );
    const billing = new Billing(store as unknown as Store, provider);
    await expect(
      billing.customer(
        { id: "user", email: "user@example.com" },
        "Maria",
        "12345678901",
      ),
    ).rejects.toMatchObject({ status: 409 });
    expect(state).toBe("UNKNOWN");
    await expect(
      billing.customer(
        { id: "user", email: "user@example.com" },
        "Maria",
        "98765432100",
      ),
    ).rejects.toMatchObject({ code: "CUSTOMER_UNKNOWN" });
    expect(posts).toBe(1);
    expect(state).toBe("UNKNOWN");
  });
  it("definitive refund rejection releases admin reservation but never revokes credit", async () => {
    const f = fixture();
    f.order.status = "PAID_AWAITING_SERVICE";
    f.order.payment_id = "payment";
    f.order.refund_state = "REQUESTED";
    const store = {
      ...f.store,
      claimOrder: async () => {
        f.order.refund_state = "CREATING";
        return true;
      },
      releaseRefund: async () => {
        f.order.refund_state = "REQUESTED";
      },
    };
    const provider = new Asaas(
      { ...getConfig(), asaasApiKey: "fixture" },
      async (_input, init) =>
        init?.method === "POST"
          ? Response.json(
              {
                errors: [
                  {
                    code: "invalid_action",
                    description: "Insufficient balance",
                  },
                ],
              },
              { status: 400 },
            )
          : Response.json({
              id: "payment",
              customer: "customer",
              externalReference: "order",
              value: 29,
              billingType: "PIX",
              status: "RECEIVED",
            }),
    );
    await expect(
      new Billing(store as unknown as Store, provider).refund("order", "admin"),
    ).rejects.toMatchObject({ code: "PROVIDER_REJECTED" });
    expect(f.order.refund_state).toBe("REQUESTED");
    expect(f.order.status).toBe("PAID_AWAITING_SERVICE");
  });
});
