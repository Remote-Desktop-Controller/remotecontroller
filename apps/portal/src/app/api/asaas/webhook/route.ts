import { webhook, webhookHealth } from "@/lib/handlers";
import { route } from "@/lib/http";
export const runtime = "nodejs";
export const POST = route(webhook);
export const GET = route(webhookHealth);
