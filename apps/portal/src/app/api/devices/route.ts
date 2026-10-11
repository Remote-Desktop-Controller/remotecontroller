import { devices } from "@/lib/handlers";
import { route } from "@/lib/http";
export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export const GET = route(devices);
