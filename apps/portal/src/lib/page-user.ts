import "server-only";
import { redirect } from "next/navigation";
import { authenticatedUser } from "./supabase";
import { HttpError } from "./http";
import { safeNext } from "./auth-session";

export async function requirePageUser(next: string) {
  try {
    return await authenticatedUser({ readOnly: true });
  } catch (error) {
    if (error instanceof HttpError && [401, 403].includes(error.status)) {
      redirect("/login?next=" + encodeURIComponent(safeNext(next)) +
        (error.status === 403 ? "&error=confirmation" : ""));
    }
    throw error;
  }
}
