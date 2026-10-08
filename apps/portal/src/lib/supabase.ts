import "server-only";
import { createServerClient } from "@supabase/ssr";
import { createClient } from "@supabase/supabase-js";
import { cookies } from "next/headers";
import { getConfig } from "./config";
import { fail } from "./http";
export async function sessionClient() {
  const c = getConfig();
  if (!c.supabaseUrl || !c.supabaseAnonKey)
    fail(503, "CONFIGURATION", "Autenticação ainda não configurada.");
  const jar = await cookies();
  return createServerClient(c.supabaseUrl, c.supabaseAnonKey, {
    cookies: {
      getAll: () => jar.getAll(),
      setAll: (items) => {
        for (const { name, value, options } of items)
          jar.set(name, value, {
            ...options,
            httpOnly: true,
            secure: !c.portalOrigin?.startsWith("http:"),
            sameSite: "lax",
          });
      },
    },
  });
}
export function serviceClient() {
  const c = getConfig();
  if (!c.supabaseUrl || !c.supabaseServiceKey)
    fail(503, "CONFIGURATION", "Banco ainda não configurado.");
  return createClient(c.supabaseUrl, c.supabaseServiceKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  });
}
export async function authenticatedUser() {
  const client = await sessionClient();
  const { data, error } = await client.auth.getUser();
  if (error || !data.user) fail(401, "UNAUTHENTICATED", "Entre na sua conta.");
  if (!data.user.email_confirmed_at || !data.user.email)
    fail(403, "EMAIL_NOT_VERIFIED", "Confirme seu e-mail antes de continuar.");
  return data.user;
}
export async function administrator() {
  const user = await authenticatedUser();
  if (!getConfig().adminUserIds.includes(user.id))
    fail(403, "ADMIN_REQUIRED", "Acesso restrito à administração.");
  return user;
}
