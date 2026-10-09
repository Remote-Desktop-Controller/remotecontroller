import { afterEach, expect, it, vi } from "vitest";
import { Billing } from "../src/lib/billing";
import { Asaas } from "../src/lib/asaas";
import { getConfig } from "../src/lib/config";
import type { Store } from "../src/lib/store";

afterEach(() => vi.unstubAllEnvs());
const subject = "11111111-1111-4111-8111-111111111111";
function fixture() {
  const requests: {name:string;args:Record<string,unknown>}[] = [];
  const store = {
    hasActiveTestGrant: async () => false,
    authorizationOrders: async () => [],
    rpc: async (name:string,args:Record<string,unknown>) => {
      requests.push({name,args});
      return {subject, allowed:true, plan:"FREE", remaining_seconds:1800};
    },
  };
  const billing = new Billing(store as unknown as Store, new Asaas(getConfig(), async () => {
    throw new Error("Free must never contact payment provider");
  }));
  return {billing,requests,store};
}
it("inspects free balance without provider calls or quota reservation", async () => {
  const {billing,requests} = fixture(); const op = crypto.randomUUID();
  expect(await billing.usage(subject,op,"inspect",120,true)).toMatchObject({plan:"FREE"});
  expect(requests).toEqual([{name:"portal_usage",args:{p_user:subject,p_operation:op,p_action:"inspect",p_max_seconds:120,p_available:true,p_verified_orders:[]}}]);
});
it("completion releases reservations even during a payment-provider outage",async()=>{
  const {billing,store,requests}=fixture();
  store.authorizationOrders=async()=>{throw new Error("provider unavailable");};
  await billing.usage(subject,crypto.randomUUID(),"complete",120,true);
  expect(requests[0].args).toMatchObject({p_action:"complete",p_verified_orders:[]});
});
it("database outage cannot grant free access",async()=>{
  const {billing,store}=fixture();
  store.rpc=async()=>{throw new Error("database unavailable");};
  await expect(billing.usage(subject,crypto.randomUUID(),"reserve",120,true)).rejects.toThrow("database unavailable");
});
it("confirmed pre-dispatch rejection can void the reservation without provider I/O",async()=>{
  const {billing,store,requests}=fixture();
  store.authorizationOrders=async()=>{throw new Error("provider unavailable");};
  await billing.usage(subject,crypto.randomUUID(),"abort",120,true);
  expect(requests[0].args).toMatchObject({p_action:"abort",p_verified_orders:[]});
});
it("premium checkout always reserves R$9.90 even with stale price environment",async()=>{
  vi.stubEnv("PORTAL_PRICE_CENTS","2990");
  expect(getConfig().priceCents).toBe(990);
});
it("free authorization remains gated until explicitly activated after migrations",async()=>{
  vi.stubEnv("FREE_DAILY_ENABLED","false");
  expect(getConfig().freeDailyEnabled).toBe(false);
  vi.stubEnv("FREE_DAILY_ENABLED","true");
  expect(getConfig().freeDailyEnabled).toBe(true);
});
