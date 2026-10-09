import { createServerClient } from "@supabase/ssr";
import { NextResponse, type NextRequest } from "next/server";
import { getConfig } from "@/lib/config";

// Cookie refresh only. The page and APIs independently validate verified identity.
export async function proxy(request: NextRequest) {
  const c = getConfig();
  let response = NextResponse.next({ request });
  if (c.supabaseUrl && c.supabaseAnonKey) {
    const client = createServerClient(c.supabaseUrl, c.supabaseAnonKey, {
      cookies: {
        getAll: () => request.cookies.getAll(),
        setAll: (items) => {
          for (const { name, value } of items) request.cookies.set(name, value);
          response = NextResponse.next({ request });
          for (const { name, value, options } of items) response.cookies.set(name, value, {
            ...options, httpOnly: true, secure: !c.portalOrigin?.startsWith("http:"), sameSite: "lax",
          });
        },
      },
    });
    try { await client.auth.getUser(); } catch { /* The page will deny unverifiable identity. */ }
  }
  response.headers.set("Cache-Control", "private, no-store");
  response.headers.set("Pragma", "no-cache");
  response.headers.set("Expires", "0");
  return response;
}

export const config = { matcher: ["/dashboard/:path*"] };
