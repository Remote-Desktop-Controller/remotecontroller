import { PGlite } from "@electric-sql/pglite";
import { readFileSync, readdirSync } from "node:fs";
import { beforeEach, afterEach, describe, it, expect } from "vitest";
import { Billing } from "../src/lib/billing";
import { Asaas } from "../src/lib/asaas";
import { getConfig } from "../src/lib/config";
import type { Store, Order } from "../src/lib/store";

const testUser = "33333333-3333-4333-8333-333333333333";
let db: PGlite;

async function rpc(name: string, args: Record<string, unknown>) {
  const entries = Object.entries(args);
  return (
    await db.query<{ result: unknown }>(
      `select ${name}(${entries.map(([k], i) => `${k} => $${i + 1}`).join(",")}) result`,
      entries.map(([, v]) => v),
    )
  ).rows[0].result;
}

function createBilling() {
  const store = {
    rpc,
    hasActiveTestGrant: async (subject: string) => {
      const res = await db.query(
        "select 1 from public.portal_test_grants where user_id = $1 and expires_at > now()",
        [subject],
      );
      return res.rows.length > 0;
    },
    authorizationOrders: async (subject: string) =>
      (
        await db.query<Order>(
          `select o.* from portal_orders o join portal_entitlements e on e.order_id=o.id where o.user_id=$1 and e.revoked_at is null and (e.activated_at is null or e.expires_at>now()) order by o.created_at,o.id`,
          [subject],
        )
      ).rows,
  };
  return new Billing(
    store as unknown as Store,
    new Asaas({ ...getConfig(), asaasApiKey: "fixture" }, async () => {
      throw new Error("Asaas API must NEVER be called for test grants");
    }),
  );
}

beforeEach(async () => {
  db = new PGlite();
  await db.exec(
    `create role anon;create role authenticated;create role service_role bypassrls;create schema auth;create table auth.users(id uuid primary key);insert into auth.users values('${testUser}');create function auth.uid() returns uuid language sql stable as $$select null::uuid$$;`,
  );
  const directory = new URL("../supabase/migrations/", import.meta.url);
  for (const file of readdirSync(directory).filter((f) => f.endsWith(".sql")).sort()) {
    await db.exec(readFileSync(new URL(file, directory), "utf8"));
  }
}, 30000);

afterEach(async () => {
  await db.close();
});

describe("administrative test grants", () => {
  it("authorizes execution with active test grant without touching orders or entitlements", async () => {
    // 1. Inserir direito de teste administrativo puro
    await db.query(
      `insert into public.portal_test_grants (user_id, label, expires_at)
       values ($1, 'TESTE_ADMINISTRATIVO', now() + interval '7 days')`,
      [testUser],
    );

    const b = createBilling();
    const op = crypto.randomUUID();

    // 2. Executar autorização (disponibilidade true)
    const authResult = (await b.authorize(testUser, op, "execute", true)) as {
      allowed: boolean;
      state: string;
      is_test: boolean;
      expires_at: number;
    };

    expect(authResult.allowed).toBe(true);
    expect(authResult.state).toBe("ACTIVE");
    expect(authResult.is_test).toBe(true);
    expect(authResult.expires_at).toBeGreaterThan(Math.floor(Date.now() / 1000));

    // 3. Garantir que ZERO pedidos ou entitlements pagos foram criados
    const orders = (await db.query("select * from portal_orders")).rows;
    expect(orders).toHaveLength(0);

    const entitlements = (await db.query("select * from portal_entitlements")).rows;
    expect(entitlements).toHaveLength(0);

    // 4. Operação registrada com entitlement_id null
    const operations = (
      await db.query<{ user_id: string; entitlement_id: string | null }>(
        "select * from portal_operations",
      )
    ).rows;
    expect(operations).toHaveLength(1);
    expect(operations[0].entitlement_id).toBeNull();
  });

  it("does not activate queued paid entitlements when test grant is active", async () => {
    // 1. Simular que o usuário tem um pedido pago de 30 dias aguardando ativação (PAID_AWAITING_SERVICE)
    const orderId = crypto.randomUUID();
    await db.query(
      `insert into portal_orders(id, user_id, amount_cents, status, creation_state)
       values ($1, $2, 990, 'PAID_AWAITING_SERVICE', 'READY')`,
      [orderId, testUser],
    );
    await db.query(
      `insert into portal_entitlements(user_id, order_id) values ($1, $2)`,
      [testUser, orderId],
    );

    // 2. Inserir também a concessão de teste
    await db.query(
      `insert into public.portal_test_grants (user_id, label, expires_at)
       values ($1, 'TESTE_ADMINISTRATIVO', now() + interval '7 days')`,
      [testUser],
    );

    const b = createBilling();
    const op = crypto.randomUUID();

    // 3. Autorizar comando
    const authResult = (await b.authorize(testUser, op, "execute", true)) as {
      allowed: boolean;
      is_test: boolean;
    };
    expect(authResult.allowed).toBe(true);
    expect(authResult.is_test).toBe(true);

    // 4. O entitlement pago NÃO pode ter sido ativado (activated_at deve continuar null!)
    const ent = (
      await db.query<{ activated_at: string | null }>(
        "select activated_at from portal_entitlements where order_id = $1",
        [orderId],
      )
    ).rows[0];
    expect(ent.activated_at).toBeNull();
  });

  it("rejects authorization when test grant is expired and no verified order exists", async () => {
    // Inserir concessão expirada
    await db.query(
      `insert into public.portal_test_grants (user_id, label, expires_at)
       values ($1, 'TESTE_EXPIRADO', now() - interval '1 hour')`,
      [testUser],
    );

    const b = createBilling();
    const authResult = (await b.authorize(testUser, crypto.randomUUID(), "execute", true)) as {
      allowed: boolean;
    };
    expect(authResult.allowed).toBe(false);
  });
});
