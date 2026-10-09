import { usage } from "@/lib/handlers";
import { route } from "@/lib/http";
export const runtime = "nodejs";
export const POST = route(usage);
