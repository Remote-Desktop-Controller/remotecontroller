import { PGlite } from "@electric-sql/pglite";
import { readFileSync, existsSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const a = "11111111-1111-4111-8111-111111111111",
  b = "22222222-2222-4222-8222-222222222222";
let db: PGlite;
type LedgerRow = {
  name: string | null;
  activated_at: string | null;
  n: number;
  seconds: number;
  id: string;
  state: string;
  allowed: boolean;
  subject: string;
};
async function rpc(name: string, args: unknown[] = []) {
  const r = await db.query<{ result: LedgerRow }>(
    `select ${name}(${args.map((_, i) => `$${i + 1}`).join(",")}) as result`,
    args,
  );
  return r.rows[0].result;
}
async function paid(user = a) {
  const order = await rpc("portal_checkout_reserve", [user, 2900]);
  await rpc("portal_checkout_complete", [
    order.id,
    "cus_test",
    "pay_" + order.id,
    "https://sandbox.asaas.com/i/test",
  ]);
  await rpc("portal_payment_apply", [order.id, "evt_" + order.id, "RECEIVED"]);
  return order.id;
}
beforeEach(async () => {
  db = new PGlite();
  await db.exec(
    `create role anon; create role authenticated; create role service_role bypassrls; create schema auth; create table auth.users(id uuid primary key); insert into auth.users values ('${a}'),('${b}'); create function auth.uid() returns uuid language sql stable as $$ select nullif(current_setting('request.jwt.claim.sub',true),'')::uuid $$;`,
  );
  const path = new URL(
    "../supabase/migrations/202610080001_commercial.sql",
    import.meta.url,
  );
  if (existsSync(path)) await db.exec(readFileSync(path, "utf8"));
}, 30000);
afterEach(async () => {
  await db.close();
});
describe("commercial SQL ledger", () => {
  it("installs PostgreSQL authority tables", async () => {
    const r = await db.query<LedgerRow>(
      "select to_regclass('public.portal_orders') as name",
    );
    expect(r.rows[0].name).toBe("portal_orders");
  });
  it("settles once, never on CONFIRMED, and inspect cannot activate", async () => {
    const o = await rpc("portal_checkout_reserve", [a, 2900]);
    await rpc("portal_checkout_complete", [
      o.id,
      "cus_test",
      "pay_test",
      "https://sandbox.asaas.com/i/test",
    ]);
    await rpc("portal_payment_apply", [o.id, "confirmed", "CONFIRMED"]);
    expect((await rpc("portal_subscription", [a, false])).state).toBe(
      "PENDING_PAYMENT",
    );
    await rpc("portal_payment_apply", [o.id, "received", "RECEIVED"]);
    await rpc("portal_payment_apply", [o.id, "received", "RECEIVED"]);
    await rpc("portal_payment_apply", [o.id, "received_again", "RECEIVED"]);
    expect(
      (await db.query<LedgerRow>("select * from portal_entitlements")).rows,
    ).toHaveLength(1);
    expect((await rpc("portal_subscription", [a, false])).state).toBe(
      "PAID_AWAITING_SERVICE",
    );
    const inspected = await rpc("portal_authorize", [
      a,
      crypto.randomUUID(),
      "inspect",
      true,
    ]);
    expect(inspected.allowed).toBe(false);
    expect(inspected.state).toBe("READY_TO_ACTIVATE");
    expect(
      (
        await db.query<LedgerRow>(
          "select activated_at from portal_entitlements",
        )
      ).rows[0].activated_at,
    ).toBeNull();
  });
  it("activates exactly 2592000 seconds, serializes concurrent calls and expired retries never burn next credit", async () => {
    await paid();
    await paid();
    const operation = crypto.randomUUID();
    const results = await Promise.all([
      rpc("portal_authorize", [a, operation, "execute", true]),
      rpc("portal_authorize", [a, operation, "execute", true]),
    ]);
    expect(results.every((x) => x.allowed)).toBe(true);
    expect(
      (
        await db.query<LedgerRow>(
          "select count(*)::int as n from portal_entitlements where activated_at is not null",
        )
      ).rows[0].n,
    ).toBe(1);
    expect(
      (
        await db.query<LedgerRow>(
          "select extract(epoch from expires_at-activated_at)::int as seconds from portal_entitlements where activated_at is not null",
        )
      ).rows[0].seconds,
    ).toBe(2592000);
    await db.exec(
      "update portal_entitlements set activated_at=now()-interval '31 days',expires_at=now()-interval '1 day' where activated_at is not null",
    );
    expect(
      (await rpc("portal_authorize", [a, operation, "execute", true])).allowed,
    ).toBe(false);
    expect(
      (
        await db.query<LedgerRow>(
          "select count(*)::int as n from portal_entitlements where activated_at is null",
        )
      ).rows[0].n,
    ).toBe(1);
    expect(
      (
        await rpc("portal_authorize", [
          a,
          crypto.randomUUID(),
          "execute",
          false,
        ])
      ).allowed,
    ).toBe(false);
    expect(
      (await rpc("portal_authorize", [a, crypto.randomUUID(), "execute", true]))
        .allowed,
    ).toBe(true);
  });
  it("refund revokes credit permanently, even stale RECEIVED cannot resurrect", async () => {
    const id = await paid();
    await rpc("portal_payment_apply", [id, "refunded", "REFUNDED"]);
    await rpc("portal_payment_apply", [id, "stale", "RECEIVED"]);
    expect(
      (await rpc("portal_authorize", [a, crypto.randomUUID(), "execute", true]))
        .allowed,
    ).toBe(false);
    expect((await rpc("portal_subscription", [a, true])).state).toBe(
      "REFUNDED",
    );
  });
  it("cancellation racing settlement never discards real paid credit", async () => {
    const o = await rpc("portal_checkout_reserve", [a, 2900]);
    await rpc("portal_checkout_complete", [
      o.id,
      "cus_test",
      "pay_test",
      "https://sandbox.asaas.com/i/test",
    ]);
    await rpc("portal_payment_apply", [o.id, "cancel", "DELETED"]);
    await rpc("portal_payment_apply", [o.id, "settlement", "RECEIVED"]);
    await rpc("portal_payment_apply", [o.id, "late-cancel", "DELETED"]);
    expect((await rpc("portal_subscription", [a, false])).state).toBe(
      "PAID_AWAITING_SERVICE",
    );
    expect(
      (await db.query<LedgerRow>("select * from portal_entitlements")).rows,
    ).toHaveLength(1);
  });
  it("partial effective refund revokes an active period immediately", async () => {
    const id = await paid();
    const op = crypto.randomUUID();
    expect(
      (await rpc("portal_authorize", [a, op, "execute", true])).allowed,
    ).toBe(true);
    await rpc("portal_payment_apply", [id, "partial", "REFUND_PARTIALLY_DONE"]);
    expect(
      (await rpc("portal_authorize", [a, op, "inspect", true])).allowed,
    ).toBe(false);
    expect(
      (await rpc("portal_authorize", [a, op, "execute", true])).allowed,
    ).toBe(false);
  });
  it("RLS isolates users and forbids browser mutations and privileged RPC", async () => {
    await paid();
    await paid(b);
    await db.exec(`set role authenticated; set request.jwt.claim.sub='${a}'`);
    expect(
      (await db.query<LedgerRow>("select * from portal_orders")).rows,
    ).toHaveLength(1);
    expect(
      (await db.query<LedgerRow>("select * from portal_entitlements")).rows,
    ).toHaveLength(1);
    await expect(
      db.exec("update portal_entitlements set revoked_at=null"),
    ).rejects.toThrow();
    await expect(
      rpc("portal_authorize", [a, crypto.randomUUID(), "execute", true]),
    ).rejects.toThrow();
  });
  it("tickets are audience/state bound, expire and are single use", async () => {
    await rpc("portal_ticket_create", [
      "hash",
      a,
      "user@example.com",
      "nonce",
      "https://remote.example",
    ]);
    expect(
      await rpc("portal_ticket_exchange", [
        "hash",
        "wrong",
        "https://remote.example",
      ]),
    ).toBeNull();
    expect(
      (
        await rpc("portal_ticket_exchange", [
          "hash",
          "nonce",
          "https://remote.example",
        ])
      ).subject,
    ).toBe(a);
    expect(
      await rpc("portal_ticket_exchange", [
        "hash",
        "nonce",
        "https://remote.example",
      ]),
    ).toBeNull();
  });
  it("expired tickets and wrong audiences cannot establish identity", async () => {
    await rpc("portal_ticket_create", [
      "hash",
      a,
      "user@example.com",
      "nonce",
      "https://remote.example",
    ]);
    expect(
      await rpc("portal_ticket_exchange", [
        "hash",
        "nonce",
        "https://evil.example",
      ]),
    ).toBeNull();
    await db.exec(
      "update portal_tickets set expires_at=now()-interval '1 second'",
    );
    expect(
      await rpc("portal_ticket_exchange", [
        "hash",
        "nonce",
        "https://remote.example",
      ]),
    ).toBeNull();
  });
  it("distributed rate limit increments atomically", async () => {
    expect(await rpc("portal_rate_limit", ["checkout:" + a, 2, 60])).toBe(true);
    expect(await rpc("portal_rate_limit", ["checkout:" + a, 2, 60])).toBe(true);
    expect(await rpc("portal_rate_limit", ["checkout:" + a, 2, 60])).toBe(
      false,
    );
  });
});
