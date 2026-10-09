/** Isolated integration: real handlers, Store, Billing, Asaas HTTP client and PostgreSQL
 * functions. Auth and Supabase transport are local fixtures, not real Supabase Auth.
 * The Asaas-shaped HTTP server is a simulator; no real provider delivery is claimed. */
/* eslint-disable @typescript-eslint/no-explicit-any -- Dynamic PostgREST/RPC fixture rows. */
import { PGlite } from "@electric-sql/pglite";
import { AsyncLocalStorage } from "node:async_hooks";
import { createServer, type Server } from "node:http";
import { readFileSync, readdirSync } from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
const seam = vi.hoisted(() => ({ client: null as any, auth: null as any, user: null as any }));
vi.mock("../src/lib/supabase", () => ({
  serviceClient: () => seam.client,
  sessionClient: async () => ({ auth: seam.auth }),
  authenticatedUser: async () => seam.user(),
  administrator: async () => seam.user(),
}));
import * as handlers from "../src/lib/handlers";
import { route, HttpError } from "../src/lib/http";

const users = ["11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"];
const session = new AsyncLocalStorage<string | undefined>();
let db: PGlite, portal: Server, provider: Server, origin: string, providerOrigin: string;
const realFetch = globalThis.fetch;
type Payment = { id: string; customer: string; externalReference: string; billingType: string; value: number; status: string; invoiceUrl: string; deleted?: boolean; refunds?: {status: string; value: number}[] };
const payments = new Map<string, Payment>();
const customers = new Map<string,{id:string;externalReference:string}>();
const registered = new Set<string>();
let outage = false, providerPosts = 0;

// Deliberately small PostgREST transport emulator. It does not replace Store or SQL RPCs.
class Query implements PromiseLike<any> {
  conditions: [string, string, unknown][] = [];
  fields = "*"; mutation?: Record<string, unknown>; insertion?: Record<string, unknown>;
  singleRow = false; max = 100; start = 0; inner = false; live = false;
  constructor(readonly table: string) { if (!/^portal_[a-z_]+$/.test(table)) throw Error("table"); }
  select(fields = "*") { this.fields = fields; this.inner = fields.includes("!inner"); return this; }
  eq(k: string, v: unknown) { this.conditions.push([k,"=",v]); return this; }
  neq(k: string, v: unknown) { this.conditions.push([k,"<>",v]); return this; }
  is(k: string, v: unknown) { this.conditions.push([k,"is",v]); return this; }
  not(k: string) { this.conditions.push([k,"not",null]); return this; }
  in(k: string,v: unknown[]) { this.conditions.push([k,"in",v]); return this; }
  or() { this.live = true; return this; }
  order() { return this; }
  limit(n: number) { this.max=n; return this; }
  range(a: number,b: number) { this.start=a; this.max=b-a+1; return this; }
  update(v: Record<string,unknown>) { this.mutation=v; return this; }
  upsert(v: Record<string,unknown>) { this.insertion=v; return this; }
  single() { this.singleRow=true; return this; }
  maybeSingle() { return this.single(); }
  async run() {
    try {
      const args: unknown[]=[];
      const bind=(v: unknown) => { args.push(v); return `$${args.length}`; };
      const column=(k: string) => { if(!/^[a-z_]+(?:\.[a-z_]+)?$/.test(k)) throw Error("column"); return k.startsWith("entitlement.") ? "e."+k.split(".")[1] : "t."+k; };
      const clauses=this.conditions.map(([k,op,v]) => op==="is" ? `${column(k)} is null` : op==="not" ? `${column(k)} is not null` : op==="in" ? `${column(k)} in (${(v as unknown[]).map(bind).join(",")})` : `${column(k)} ${op} ${bind(v)}`);
      if(this.live) clauses.push("(e.activated_at is null or e.expires_at>now())");
      const where=clauses.length?" where "+clauses.join(" and "):"";
      let sql: string;
      if(this.mutation) {
        const assignments=Object.entries(this.mutation).map(([k,v])=>`${k}=${bind(v)}`);
        sql=`update ${this.table} t set ${assignments.join(",")}${where} returning t.*`;
      } else if(this.insertion) {
        const entries=Object.entries(this.insertion);
        sql=`insert into ${this.table} (${entries.map(([k])=>k).join(",")}) values (${entries.map(([,v])=>bind(v)).join(",")}) on conflict(event_key) do nothing returning *`;
      } else {
        const joined=this.fields.includes("entitlement:");
        sql=`select t.*${joined?",case when e.id is null then null else to_jsonb(e) end as entitlement":""} from ${this.table} t${joined?` ${this.inner?"inner":"left"} join portal_entitlements e on e.order_id=t.id`:""}${where}${this.table==="portal_orders"?" order by t.created_at,t.id":""} limit ${this.max} offset ${this.start}`;
      }
      const result=await db.query(sql,args);
      return {data:this.singleRow?(result.rows[0]??null):result.rows,error:null,count:result.rows.length};
    } catch(error) { return {data:null,error}; }
  }
  then<TResult1 = any, TResult2 = never>(onfulfilled?: ((value:any)=>TResult1|PromiseLike<TResult1>) | null, onrejected?: ((reason:any)=>TResult2|PromiseLike<TResult2>) | null): PromiseLike<TResult1|TResult2> { return this.run().then(onfulfilled,onrejected); }
}
async function listen(server: Server) {
  await new Promise<void>(resolve=>server.listen(0,"127.0.0.1",resolve));
  return "http://127.0.0.1:"+(server.address() as {port:number}).port;
}
async function close(server: Server) { await new Promise<void>((resolve,reject)=>server.close(e=>e?reject(e):resolve())); }
async function request(path: string, body?: unknown, user=users[0], extra: Record<string,string>={}) {
  const response=await realFetch(origin+path,{method:body===undefined?"GET":"POST",headers:{origin,"content-type":"application/json","x-fixture-user":user,...extra},body:body===undefined?undefined:JSON.stringify(body),redirect:"manual"});
  return {status:response.status,body:await response.json()};
}
async function buy(user=users[0]) {
  const result=await request("/api/checkout",{name:"Fixture User",cpfCnpj:"12345678901",termsAccepted:true},user);
  expect(result.status).toBe(200);
  return {id:result.body.orderId as string,payment:[...payments.values()].find(p=>p.externalReference===result.body.orderId)!};
}
async function notify(p: Payment,event="PAYMENT_RECEIVED",id=crypto.randomUUID(),token="w".repeat(32)) {
  return request("/api/asaas/webhook",{id,event,payment:{id:p.id,externalReference:p.externalReference}},users[0],{"asaas-access-token":token});
}
async function authorize(user=users[0],mode="execute",operation=crypto.randomUUID(),token="s".repeat(32)) {
  return request("/api/mcp/authorize",{subject:user,operation_id:operation,mode},user,{authorization:"Bearer "+token});
}
async function ledger() { return (await db.query<any>("select *,extract(epoch from(expires_at-activated_at))::int as seconds from portal_entitlements order by created_at")).rows; }
async function rustRoundTrip() {
  return new Promise<string>((resolve,reject)=>{
    const child=spawn(process.env.RDC_E2E_RUST_TEST_BIN!,["--ignored","--nocapture"],{cwd:fileURLToPath(new URL("../../../",import.meta.url)),env:{...process.env,RDC_E2E_PORTAL_ORIGIN:origin},windowsHide:true});
    let text="";child.stdout.on("data",chunk=>text+=chunk);child.stderr.on("data",chunk=>text+=chunk);
    const timer=setTimeout(()=>{child.kill();reject(Error("Rust crossstack timed out\n"+text));},240000);
    child.on("error",error=>{clearTimeout(timer);reject(error);});
    child.on("exit",code=>{clearTimeout(timer);if(code!==0)reject(Error("Rust crossstack exit "+code+"\n"+text));else resolve(text);});
  });
}

beforeAll(async()=> {
  db=new PGlite();
  await db.exec(`create role anon; create role authenticated; create role service_role bypassrls; create schema auth; create table auth.users(id uuid primary key); insert into auth.users values ('${users[0]}'),('${users[1]}'); create function auth.uid() returns uuid language sql stable as $$ select nullif(current_setting('request.jwt.claim.sub',true),'')::uuid $$;`);
  const migrations=new URL("../supabase/migrations/",import.meta.url);
  for(const file of readdirSync(migrations).filter(f=>f.endsWith(".sql")).sort()) await db.exec(readFileSync(new URL(file,migrations),"utf8"));
  seam.client={from:(table:string)=>new Query(table),rpc:async(name:string,args:Record<string,unknown>)=> {
    try { const pairs=Object.entries(args); const sql=`select ${name}(${pairs.map(([k],i)=>`${k} => $${i+1}`).join(",")}) as result`; return {data:(await db.query<any>(sql,pairs.map(([,v])=>v))).rows[0].result,error:null}; } catch(error) { return {data:null,error}; }
  }};
  seam.user=()=> {const id=session.getStore(); if(!id||!users.includes(id)) throw new HttpError(401,"UNAUTHENTICATED","Login required"); return {id,email:id+"@fixture.test",email_confirmed_at:new Date().toISOString()};};
  seam.auth={signUp:async({email}:{email:string})=>{registered.add(email);return {error:null};},signInWithPassword:async({email,password}:{email:string,password:string})=>({data:{user:registered.has(email)&&password==="fixture-password-123"?{email,email_confirmed_at:"2026-01-01"}:null},error:null}),signOut:async()=>({error:null})};
  provider=createServer(async(req,res)=> {
    res.setHeader("Content-Type","application/json");
    if(outage) {res.statusCode=503;res.end("{}");return;}
    let raw=""; for await(const chunk of req) raw+=chunk;
    const body=raw?JSON.parse(raw):{};
    const url=new URL(req.url!,"http://fixture");
    if(req.method==="POST") providerPosts++;
    let result: unknown={};
    if(req.method==="POST"&&url.pathname==="/v3/customers") { const customer={id:"cus_"+body.externalReference,externalReference:body.externalReference};customers.set(customer.id,customer);result=customer; }
    else if(req.method==="POST"&&url.pathname==="/v3/payments") {const p={...body,id:"pay_"+crypto.randomUUID(),status:"PENDING",invoiceUrl:"https://sandbox.asaas.com/i/fixture"};payments.set(p.id,p);result=p;}
    else if(req.method==="GET"&&url.pathname==="/v3/customers") result={data:[...customers.values()].filter(c=>c.externalReference===url.searchParams.get("externalReference")),hasMore:false};
    else if(req.method==="GET"&&url.pathname==="/v3/payments") result={data:[...payments.values()].filter(p=>p.externalReference===url.searchParams.get("externalReference")),hasMore:false};
    else {const id=url.pathname.split("/")[3];const p=payments.get(id);if(!p){res.statusCode=404;res.end("{}");return;}if(req.method==="DELETE")p.deleted=true;result=p;}
    res.end(JSON.stringify(result));
  });
  providerOrigin=await listen(provider);
  const routes: Record<string,any>={"/api/auth/register":handlers.register,"/api/auth/login":handlers.login,"/api/account":handlers.account,"/api/checkout":handlers.checkout,"/api/asaas/webhook":handlers.webhook,"/api/mcp/authorize":handlers.authorize,"/api/mcp/usage":handlers.usage,"/api/mcp/connect":handlers.connect,"/api/mcp/exchange":handlers.exchange};
  portal=createServer(async(req,res)=> {
    let raw="";for await(const chunk of req) raw+=chunk;
    await session.run(req.headers["x-fixture-user"] as string,async()=> {
      const path=new URL(req.url!,origin).pathname;
      const match=path.match(/^\/api\/orders\/([^/]+)\/(sync|cancel)$/);
      const handler=routes[path]??(match?.[2]==="sync"?handlers.sync:match?handlers.cancel:undefined);
      if(!handler){res.statusCode=404;res.end("{}");return;}
      const response=await route(handler)(new Request(origin+req.url,{method:req.method,headers:req.headers as Record<string,string>,body:req.method==="POST"?raw:undefined}),{params:Promise.resolve({id:match?.[1]})});
      res.statusCode=response.status;response.headers.forEach((v,k)=>res.setHeader(k,v));res.end(await response.text());
    });
  });
  origin=await listen(portal);
},30000);
beforeEach(async()=> {
  await db.exec("truncate portal_usage_operations,portal_usage_sessions,portal_test_grants restart identity cascade");
  await db.exec("truncate portal_operations,portal_tickets,portal_events,portal_entitlements,portal_orders,portal_customers,portal_rate_limits restart identity cascade");
  payments.clear();customers.clear();registered.clear();outage=false;providerPosts=0;
  for(const [key,value] of Object.entries({FREE_DAILY_ENABLED:"false",PORTAL_PUBLIC_URL:origin,PORTAL_ALLOW_HTTP_LOOPBACK:"true",MCP_SERVICE_ORIGIN:"https://remote.fixture.test",MCP_SHARED_SECRET:"s".repeat(32),ASAAS_WEBHOOK_TOKEN:"w".repeat(32),ASAAS_API_KEY:"fixture-only",ASAAS_ENV:"sandbox",PORTAL_PRICE_CENTS:"990",SALES_ENABLED:"true",REMOTE_SERVICE_AVAILABLE:"false"})) vi.stubEnv(key,value);
  vi.stubGlobal("fetch",((input: string|URL|Request,init?:RequestInit)=> {
    const url=String(input);
    if(!url.startsWith("https://api-sandbox.asaas.com/v3/")) throw Error("Unexpected external network blocked: "+url);
    return realFetch(url.replace("https://api-sandbox.asaas.com",providerOrigin),init);
  }) as typeof fetch);
});
afterAll(async()=> {vi.unstubAllGlobals();vi.unstubAllEnvs();await close(portal);await close(provider);await db.close();});

describe("isolated commercial lifecycle over HTTP and real PostgreSQL",()=> {
  it("runs Free over authenticated HTTP with no payment and no consumption on inspection",async()=>{
    vi.stubEnv("FREE_DAILY_ENABLED","true");vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");
    const op=crypto.randomUUID(),headers={authorization:"Bearer "+"s".repeat(32)};
    expect((await request("/api/mcp/usage",{subject:users[0],operation_id:op,action:"inspect"},users[0],headers)).body).toMatchObject({allowed:true,plan:"FREE",remaining_seconds:1800});
    expect((await request("/api/account")).body.usage).toMatchObject({plan:"FREE",remaining_seconds:1800});
    const reserved=await request("/api/mcp/usage",{subject:users[0],operation_id:op,action:"reserve",max_seconds:10},users[0],headers);
    expect(reserved.body).toMatchObject({allowed:true,plan:"FREE"});
    const replay=await request("/api/mcp/usage",{subject:users[0],operation_id:op,action:"reserve",max_seconds:120},users[0],headers);
    expect(replay.body.expires_at).toBe(reserved.body.expires_at);
    expect((await request("/api/mcp/usage",{subject:users[0],operation_id:op,action:"complete"},users[0],headers)).status).toBe(200);
    expect(providerPosts).toBe(0);expect(await ledger()).toHaveLength(0);
    expect((await db.query("select * from portal_orders")).rows).toHaveLength(0);
  });
  it("overlaps HTTP checkout/settlement/use across users and reserves one charge per repeated checkout",async()=> {
    // Requests overlap, but PGlite is one SQL connection: this is not distributed PG contention proof.
    const [a,b]=await Promise.all([buy(users[0]),buy(users[1])]);
    a.payment.status="RECEIVED";b.payment.status="RECEIVED";
    const settled=await Promise.all([notify(a.payment),notify(b.payment),notify(a.payment,"PAYMENT_RECEIVED","overlap-duplicate"),notify(a.payment,"PAYMENT_RECEIVED","overlap-duplicate")]);
    expect(settled.every(r=>r.status===200)).toBe(true);
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");
    const results=await Promise.all([authorize(users[0]),authorize(users[1])]);
    expect(results.map(r=>r.body.allowed)).toEqual([true,true]);
    const rights=await ledger();expect(rights).toHaveLength(2);expect(new Set(rights.map(r=>r.user_id))).toEqual(new Set(users));expect(rights.every(r=>r.seconds===2592000)).toBe(true);
    const priorPosts=providerPosts;
    const checkoutBody={name:"Fixture User",cpfCnpj:"12345678901",termsAccepted:true};
    const repeated=await Promise.all([request("/api/checkout",checkoutBody),request("/api/checkout",checkoutBody)]);
    expect(repeated.some(r=>r.status===200)).toBe(true);
    expect(repeated.every(r=>[200,409].includes(r.status))).toBe(true);
    expect(providerPosts-priorPosts).toBe(1);
    expect((await db.query("select * from portal_orders where user_id=$1 and status='PENDING_PAYMENT'",[users[0]])).rows).toHaveLength(1);
  });
  it.skipIf(!process.env.RDC_E2E_RUST_TEST_BIN)("joins real Rust service/device-agent HTTP and IPC to the actual portal SQL license",async()=> {
    const {payment}=await buy();payment.status="RECEIVED";await notify(payment);
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");
    vi.stubEnv("FREE_DAILY_ENABLED","true");
    expect((await ledger())[0].activated_at).toBeNull();
    const output=await rustRoundTrip();
    expect(output).toContain("1 passed");
    const rights=await ledger();expect(rights).toHaveLength(1);expect(rights[0].activated_at).not.toBeNull();expect(rights[0].seconds).toBe(2592000);
    expect((await db.query("select * from portal_operations")).rows).toHaveLength(1);
  },300000);
  it.skipIf(!process.env.RDC_E2E_RUST_TEST_BIN)("dispatches a real Free read through Rust, agent and daemon without creating revenue",async()=>{
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");vi.stubEnv("FREE_DAILY_ENABLED","true");
    expect(await rustRoundTrip()).toContain("1 passed");
    const rows=(await db.query<{plan:string;completed_at:string|null}>("select plan,completed_at from portal_usage_operations")).rows;
    expect(rows).toHaveLength(1);expect(rows[0].plan).toBe("FREE");expect(rows[0].completed_at).not.toBeNull();
    expect(await ledger()).toHaveLength(0);expect((await db.query("select * from portal_orders")).rows).toHaveLength(0);
    expect((await db.query("select * from portal_events")).rows).toHaveLength(0);expect(providerPosts).toBe(0);
  },300000);
  it("runs registration/login, preserves blocked sales, and isolates user orders",async()=> {
    const credentials={email:"user@fixture.test",password:"fixture-password-123"};
    expect((await request("/api/auth/register",credentials)).body.emailVerificationRequired).toBe(true);
    expect((await request("/api/auth/login",{...credentials,password:"wrong"})).status).toBe(401);
    expect((await request("/api/auth/login",credentials)).status).toBe(200);
    expect((await request("/api/account",undefined,"")).status).toBe(401);
    vi.stubEnv("SALES_ENABLED","false");
    expect((await request("/api/checkout",{name:"User",cpfCnpj:"12345678901",termsAccepted:true})).status).toBe(503);
    expect(providerPosts).toBe(0);
    vi.stubEnv("SALES_ENABLED","true");
    const order=await buy();await buy(users[1]);
    expect((await request(`/api/orders/${order.id}/sync`,{},users[1])).status).toBe(403);
    expect((await request("/api/account",undefined,users[1])).body.orders).toHaveLength(1);
    expect((await authorize(users[1])).body.allowed).toBe(false);
  });
  it("authenticates and reconciles duplicates/out-of-order events, starts exactly 30 days only on execute",async()=> {
    const {payment}=await buy();
    expect((await notify(payment,"PAYMENT_RECEIVED",crypto.randomUUID(),"invalid")).status).toBe(401);
    payment.status="CONFIRMED";expect((await notify(payment,"PAYMENT_CONFIRMED")).status).toBe(200);
    expect(await ledger()).toHaveLength(0);
    payment.status="RECEIVED";const event=crypto.randomUUID();await notify(payment,"PAYMENT_RECEIVED",event);await notify(payment,"PAYMENT_RECEIVED",event);await notify(payment,"PAYMENT_CONFIRMED");
    expect(await ledger()).toHaveLength(1);expect((await ledger())[0].activated_at).toBeNull();
    expect((await request("/api/account")).body.subscription.state).toBe("PAID_AWAITING_SERVICE");
    expect((await ledger())[0].activated_at).toBeNull();
    expect((await authorize()).body.state).toBe("PAID_AWAITING_SERVICE");
    vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");
    expect((await authorize(users[0],"inspect")).body.state).toBe("READY_TO_ACTIVATE");expect((await ledger())[0].activated_at).toBeNull();
    const operation=crypto.randomUUID();expect((await authorize(users[0],"execute",operation)).body.allowed).toBe(true);
    const first=(await ledger())[0];expect(first.seconds).toBe(2592000);
    expect((await authorize(users[0],"execute",operation)).body.allowed).toBe(true);expect((await ledger())[0].activated_at).toEqual(first.activated_at);
    expect((await authorize(users[0],"execute",crypto.randomUUID(),"wrong")).status).toBe(401);
  });
  it("fails closed during provider outage, rejects forged canonical data, and retries safely",async()=> {
    const {payment}=await buy();payment.status="RECEIVED";outage=true;
    expect((await notify(payment)).status).toBe(503);expect(await ledger()).toHaveLength(0);
    outage=false;await notify(payment);vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");outage=true;
    expect((await authorize()).status).toBe(503);expect((await ledger())[0].activated_at).toBeNull();
    outage=false;payment.value=0.01;expect((await authorize()).status).toBe(409);expect((await ledger())[0].activated_at).toBeNull();
    payment.value=9.9;expect((await authorize()).body.allowed).toBe(true);
    expect(providerPosts).toBe(2);
  });
  it("detects silent partial refund without its webhook, keeps revocation sticky, and handles total refund/cancel",async()=> {
    const first=await buy();first.payment.status="RECEIVED";await notify(first.payment);vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");await authorize();
    first.payment.refunds=[{status:"DONE",value:0.01}];
    expect((await authorize()).body.allowed).toBe(false);expect((await ledger())[0].revoked_at).not.toBeNull();
    delete first.payment.refunds;await notify(first.payment);expect((await authorize()).body.allowed).toBe(false);
    const second=await buy();second.payment.status="RECEIVED";await notify(second.payment);second.payment.status="REFUNDED";await notify(second.payment,"PAYMENT_REFUNDED");expect((await ledger()).every(e=>e.revoked_at)).toBe(true);
    const pending=await buy();expect((await request(`/api/orders/${pending.id}/cancel`,{})).status).toBe(200);expect((await db.query<any>("select status from portal_orders where id=$1",[pending.id])).rows[0].status).toBe("CANCELED");
  });
  it("renews after expiry without burning prepaid credit on replay and consumes connection tickets once",async()=> {
    const first=await buy();first.payment.status="RECEIVED";await notify(first.payment);vi.stubEnv("REMOTE_SERVICE_AVAILABLE","true");const op=crypto.randomUUID();await authorize(users[0],"execute",op);
    const second=await buy();second.payment.status="RECEIVED";await notify(second.payment);expect((await ledger())[1].activated_at).toBeNull();
    await db.exec("update portal_entitlements set activated_at=now()-interval '31 days',expires_at=now()-interval '1 day' where activated_at is not null");
    expect((await authorize(users[0],"execute",op)).body.allowed).toBe(false);expect((await ledger())[1].activated_at).toBeNull();
    expect((await authorize()).body.allowed).toBe(true);expect((await ledger())[1].seconds).toBe(2592000);
    const state="fixture_state_0123456789";const connected=await request("/api/mcp/connect?state="+state+"&redirect_uri="+encodeURIComponent("https://remote.fixture.test/auth/commercial/callback"));
    expect(connected.status).toBe(200);const ticket=new URL(connected.body.redirectUrl).searchParams.get("ticket");
    const payload={ticket,state,redirect_uri:"https://remote.fixture.test/auth/commercial/callback"};const headers={authorization:"Bearer "+"s".repeat(32)};
    expect((await request("/api/mcp/exchange",payload,users[0],headers)).body.subject).toBe(users[0]);expect((await request("/api/mcp/exchange",payload,users[0],headers)).status).toBe(401);
  });
});
