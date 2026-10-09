import { PGlite } from "@electric-sql/pglite";
import { readFileSync, readdirSync } from "node:fs";
import { beforeEach, afterEach, describe, it, expect } from "vitest";
import { Billing } from "../src/lib/billing";
import { Asaas, type Payment } from "../src/lib/asaas";
import { getConfig } from "../src/lib/config";
import type { Store, Order } from "../src/lib/store";
const user = "11111111-1111-4111-8111-111111111111";
let db: PGlite, payments: Map<string, Payment>, outage: boolean, calls: number;
async function rpc(name: string, args: Record<string, unknown>) {
    const entries = Object.entries(args);
    return (await db.query<{
        result: unknown;
    }>(`select ${name}(${entries.map(([k], i) => `${k} => $${i + 1}`).join(",")}) result`, entries.map(([, v]) => v))).rows[0].result;
}
function billing() {
    const store = { rpc, authorizationOrders: async (subject: string) => (await db.query<Order>(`select o.* from portal_orders o join portal_entitlements e on e.order_id=o.id where o.user_id=$1 and e.revoked_at is null and (e.activated_at is null or e.expires_at>now()) order by o.created_at,o.id`, [subject])).rows };
    return new Billing(store as unknown as Store, new Asaas({ ...getConfig(), asaasApiKey: "fixture" }, async (url, options) => {
        calls++;
        expect(options?.method).toBe("GET");
        return outage ? new Response("", { status: 503 }) : Response.json(payments.get(decodeURIComponent(String(url).split("/").at(-1)!)));
    }));
}
async function paid() {
    const o = await rpc("portal_checkout_reserve", { p_user: user, p_amount: 2900 }) as Order, id = "pay_" + o.id;
    await rpc("portal_checkout_complete", { p_order: o.id, p_customer: "cus", p_payment: id, p_url: "https://sandbox.asaas.com/i/test" });
    payments.set(id, { id, customer: "cus", externalReference: o.id, value: 29, billingType: "PIX", status: "RECEIVED" });
    await rpc("portal_payment_apply", { p_order: o.id, p_event: "received:" + o.id, p_status: "RECEIVED" });
    return o.id;
}
beforeEach(async () => {
    db = new PGlite();
    payments = new Map();
    outage = false;
    calls = 0;
    await db.exec(`create role anon;create role authenticated;create role service_role bypassrls;create schema auth;create table auth.users(id uuid primary key);insert into auth.users values('${user}');create function auth.uid() returns uuid language sql stable as $$select null::uuid$$;`);
    const directory = new URL("../supabase/migrations/", import.meta.url);
    for (const file of readdirSync(directory).filter(f => f.endsWith(".sql")).sort())
        await db.exec(readFileSync(new URL(file, directory), "utf8"));
}, 30000);
afterEach(async () => { await db.close(); });
describe("canonical financial authorization", () => {
    it("silent partial refund revokes ACTIVE without refund webhook and stale RECEIVED cannot revive", async () => {
        const id = await paid(), op = crypto.randomUUID();
        expect(await billing().authorize(user, op, "execute", true)).toMatchObject({ allowed: true });
        payments.get("pay_" + id)!.refunds = [{ status: "DONE", value: 1 }];
        expect(await billing().authorize(user, op, "inspect", true)).toMatchObject({ allowed: false, state: "REFUNDED" });
        await rpc("portal_payment_apply", { p_order: id, p_event: "late", p_status: "RECEIVED" });
        expect(await billing().authorize(user, crypto.randomUUID(), "execute", true)).toMatchObject({ allowed: false, state: "REFUNDED" });
    });
    it("provider outage never starts the period or records an admitted operation", async () => {
        await paid();
        outage = true;
        await expect(billing().authorize(user, crypto.randomUUID(), "execute", true)).rejects.toMatchObject({ status: 503 });
        expect((await db.query("select * from portal_entitlements where activated_at is not null")).rows).toHaveLength(0);
        expect((await db.query("select * from portal_operations")).rows).toHaveLength(0);
    });
    it("canonical pending status cannot activate a locally paid entitlement", async () => {
        const id = await paid();
        payments.get("pay_" + id)!.status = "PENDING";
        await expect(billing().authorize(user, crypto.randomUUID(), "execute", true)).rejects.toMatchObject({ status: 503 });
        expect((await db.query("select * from portal_entitlements where activated_at is not null")).rows).toHaveLength(0);
    });
    it("inspect never activates and a renewal starts only on new execution after expiry", async () => {
        await paid();
        const second = await paid(), op = crypto.randomUUID();
        expect(await billing().authorize(user, op, "inspect", true)).toMatchObject({ allowed: false, state: "READY_TO_ACTIVATE" });
        expect((await db.query("select * from portal_entitlements where activated_at is not null")).rows).toHaveLength(0);
        await billing().authorize(user, op, "execute", true);
        await db.exec("update portal_entitlements set activated_at=now()-interval '31 days',expires_at=now()-interval '1 day' where activated_at is not null");
        expect(await billing().authorize(user, op, "execute", true)).toMatchObject({ allowed: false });
        expect(await billing().authorize(user, crypto.randomUUID(), "execute", true)).toMatchObject({ allowed: true });
        expect((await db.query<{
            seconds: number;
        }>("select extract(epoch from expires_at-activated_at)::int seconds from portal_entitlements where order_id=$1", [second])).rows[0].seconds).toBe(2592000);
    });
    it("disabled service does no provider I/O or activation", async () => {
        await paid();
        outage = true;
        expect(await billing().authorize(user, crypto.randomUUID(), "execute", false)).toMatchObject({ allowed: false });
        expect(calls).toBe(0);
        expect((await db.query("select * from portal_entitlements where activated_at is not null")).rows).toHaveLength(0);
    });
});
