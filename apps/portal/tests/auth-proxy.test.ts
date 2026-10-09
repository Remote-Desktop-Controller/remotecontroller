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
