import { admin } from "@/lib/handlers";
import { route } from "@/lib/http";
export const runtime = "nodejs";
export const GET = route(admin);
