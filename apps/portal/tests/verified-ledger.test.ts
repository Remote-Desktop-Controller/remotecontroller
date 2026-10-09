import { PGlite } from "@electric-sql/pglite";
import { readFileSync } from "node:fs";
import { beforeEach, afterEach, it, expect } from "vitest";
const user="11111111-1111-4111-8111-111111111111";
let db:PGlite;
async function paid() {
 const id=crypto.randomUUID();
 await db.query("insert into portal_orders(id,user_id,amount_cents,status,creation_state) values($1,$2,990,'PAID_AWAITING_SERVICE','READY')",[id,user]);
 await db.query("insert into portal_entitlements(user_id,order_id) values($1,$2)",[user,id]);
 return id;
}
async function authorize(ids:string[],operation=crypto.randomUUID()) {
 return (await db.query<{result:{allowed:boolean}}>("select portal_authorize($1,$2,'execute',true,$3::uuid[]) result",[user,operation,ids])).rows[0].result;
}
beforeEach(async()=>{
 db=new PGlite();
 await db.exec(`create role anon;create role authenticated;create role service_role bypassrls;create schema auth;create table auth.users(id uuid primary key);insert into auth.users values('${user}');create function auth.uid() returns uuid language sql stable as $$select null::uuid$$;`);
 await db.exec(readFileSync(new URL("../supabase/migrations/202610080001_commercial.sql",import.meta.url),"utf8"));
 if(!process.env.VERIFIED_LEDGER_BASELINE) await db.exec(readFileSync(new URL("../supabase/migrations/202610090001_verified_authorization.sql",import.meta.url),"utf8"));
},30000);
afterEach(async()=>{await db.close();});
it("does not consume a concurrent credit absent from the canonical verification list",async()=>{
 await paid();expect(await authorize([])).toMatchObject({allowed:false});
 expect((await db.query("select * from portal_entitlements where activated_at is not null")).rows).toHaveLength(0);
});
it("consumes only the verified order and starts exactly 2592000 seconds",async()=>{
 await paid();const verified=await paid();expect(await authorize([verified])).toMatchObject({allowed:true});
 const rows=(await db.query<{order_id:string,seconds:number}>("select order_id,extract(epoch from expires_at-activated_at)::int seconds from portal_entitlements where activated_at is not null")).rows;
 expect(rows).toEqual([{order_id:verified,seconds:2592000}]);
});
it("operation replay without its freshly verified order cannot reuse a live entitlement",async()=>{
 const id=await paid(),op=crypto.randomUUID();expect(await authorize([id],op)).toMatchObject({allowed:true});
 expect(await authorize([],op)).toMatchObject({allowed:false});
});
it("replacement RPC remains inaccessible to anon and authenticated",async()=>{
 for(const role of ['anon','authenticated']) expect((await db.query<{allowed:boolean}>("select has_function_privilege($1,'public.portal_authorize(uuid,uuid,text,boolean,uuid[])','EXECUTE') allowed",[role])).rows[0].allowed).toBe(false);
});
