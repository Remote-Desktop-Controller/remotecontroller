import { createServerClient } from "@supabase/ssr";
import { NextResponse, type NextRequest } from "next/server";
import { getConfig } from "@/lib/config";

// Refresh cookies and redirect before streaming. Page and APIs also validate identity.
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
    try {
      const { data, error } = await client.auth.getUser();
      if (error && (error.name === "AuthRetryableFetchError" || (error.status ?? 0) >= 500)) {
        const unavailable = NextResponse.json({error:"AUTH_UNAVAILABLE",message:"Não foi possível verificar sua sessão. Tente novamente."},{status:503});
        for (const cookie of response.cookies.getAll()) unavailable.cookies.set(cookie);
        response = unavailable;
      } else if (error || !data.user?.email || !data.user.email_confirmed_at) {
        const target = new URL("/login", c.portalOrigin ?? request.nextUrl.origin);
        target.searchParams.set("next", request.nextUrl.pathname + request.nextUrl.search);
        if (data.user && !data.user.email_confirmed_at) target.searchParams.set("error", "confirmation");
        const denied = NextResponse.redirect(target);
        for (const cookie of response.cookies.getAll()) denied.cookies.set(cookie);
        response = denied;
      }
    } catch {
      const unavailable = NextResponse.json({error:"AUTH_UNAVAILABLE",message:"Não foi possível verificar sua sessão. Tente novamente."},{status:503});
      for (const cookie of response.cookies.getAll()) unavailable.cookies.set(cookie);
      response = unavailable;
    }
  } else {
    response = NextResponse.json({error:"AUTH_UNAVAILABLE",message:"Autenticação ainda não configurada."},{status:503});
  }
  response.headers.set("Cache-Control", "private, no-store");
  response.headers.set("Pragma", "no-cache");
  response.headers.set("Expires", "0");
  return response;
}

export const config = { matcher: ["/dashboard/:path*"] };
