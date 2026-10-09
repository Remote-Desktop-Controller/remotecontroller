import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { NextRequest } from "next/server";
const f=vi.hoisted(()=>({create:vi.fn()}));
vi.mock("@supabase/ssr",()=>({createServerClient:f.create}));
beforeEach(()=>{vi.resetAllMocks();vi.stubEnv("NEXT_PUBLIC_SUPABASE_URL","https://fixture.supabase.co");vi.stubEnv("NEXT_PUBLIC_SUPABASE_ANON_KEY","fixture");vi.stubEnv("PORTAL_PUBLIC_URL","https://portal.example");});
afterEach(()=>vi.unstubAllEnvs());
it("refreshes cookies for both server rendering and the browser without trusting getSession",async()=>{
 f.create.mockImplementation((_url,_key,options)=>({auth:{getUser:async()=>{options.cookies.setAll([{name:"sb-fixture-auth-token",value:"refreshed-fixture",options:{path:"/",maxAge:3600}}]);return {data:{user:null},error:null};}}}));
 const {proxy}=await import("../src/proxy");
 const req=new NextRequest("https://portal.example/dashboard",{headers:{cookie:"sb-fixture-auth-token=old-fixture"}});
 const res=await proxy(req);
 expect(req.cookies.get("sb-fixture-auth-token")?.value).toBe("refreshed-fixture");
 expect(res.cookies.get("sb-fixture-auth-token")).toMatchObject({value:"refreshed-fixture",httpOnly:true,secure:true,sameSite:"lax"});
 expect(res.headers.get("cache-control")).toContain("no-store");
});
it("redirects missing session before dashboard streaming begins",async()=>{
 f.create.mockReturnValue({auth:{getUser:async()=>({data:{user:null},error:null})}});
 const {proxy}=await import("../src/proxy");
 const res=await proxy(new NextRequest("https://portal.example/dashboard"));
 expect(res.status).toBe(307);
 expect(res.headers.get("location")).toBe("https://portal.example/login?next=%2Fdashboard");
});
it("fails closed with 503 instead of claiming logout during a provider outage",async()=>{
 f.create.mockReturnValue({auth:{getUser:async()=>({data:{user:null},error:{status:503}})}});
 const {proxy}=await import("../src/proxy");
 expect((await proxy(new NextRequest("https://portal.example/dashboard"))).status).toBe(503);
});
it("lets a freshly verified account reach the separately protected dashboard",async()=>{
 f.create.mockReturnValue({auth:{getUser:async()=>({data:{user:{email:"fixture@example.invalid",email_confirmed_at:"2026-10-09T00:00:00Z"}},error:null})}});
 const {proxy}=await import("../src/proxy");
 const res=await proxy(new NextRequest("https://portal.example/dashboard"));
 expect(res.status).toBe(200);expect(res.headers.get("location")).toBeNull();
});
it("does not treat an unconfirmed session as a verified account",async()=>{
 f.create.mockReturnValue({auth:{getUser:async()=>({data:{user:{email:"fixture@example.invalid",email_confirmed_at:null}},error:null})}});
 const {proxy}=await import("../src/proxy");
 const res=await proxy(new NextRequest("https://portal.example/dashboard"));
 expect(res.status).toBe(307);expect(res.headers.get("location")).toContain("error=confirmation");
});
