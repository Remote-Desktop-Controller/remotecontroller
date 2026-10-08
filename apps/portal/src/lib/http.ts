import "server-only";
import { timingSafeEqual, createHash } from "node:crypto";
import { z } from "zod";
import { getConfig } from "./config";
export class HttpError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}
export function fail(status: number, code: string, message: string): never {
  throw new HttpError(status, code, message);
}
export async function jsonBody<T>(
  request: Request,
  schema: z.ZodType<T>,
): Promise<T> {
  if (Number(request.headers.get("content-length") ?? "0") > 16384)
    fail(413, "BODY_TOO_LARGE", "Dados excedem o limite.");
  let raw: string;
  try {
    raw = await request.text();
  } catch {
    fail(400, "INVALID_BODY", "Dados inválidos.");
  }
  if (Buffer.byteLength(raw) > 16384)
    fail(413, "BODY_TOO_LARGE", "Dados excedem o limite.");
  try {
    return schema.parse(JSON.parse(raw));
  } catch {
    fail(400, "INVALID_BODY", "Confira os dados enviados.");
  }
}
export function sameOrigin(request: Request) {
  const allowed = getConfig().portalOrigin;
  if (!allowed) fail(503, "CONFIGURATION", "Portal ainda não configurado.");
  if (request.headers.get("origin") !== allowed)
    fail(403, "ORIGIN", "Origem não autorizada.");
}
export function secureEqual(a: string | null, b: string | null) {
  if (!a || !b) return false;
  const aa = Buffer.from(a),
    bb = Buffer.from(b);
  return aa.length === bb.length && timingSafeEqual(aa, bb);
}
export function mcpAuth(request: Request) {
  const c = getConfig();
  if (!c.mcpSecret || c.mcpSecret.length < 32 || !c.remoteOrigin)
    fail(503, "CONFIGURATION", "Integração ainda não configurada.");
  if (
    !secureEqual(request.headers.get("authorization"), "Bearer " + c.mcpSecret)
  )
    fail(401, "UNAUTHORIZED", "Autenticação inválida.");
  return c;
}
export function hash(value: string) {
  return createHash("sha256").update(value).digest("hex");
}
export const uuid = z.string().uuid();
export type RouteContext = {
  params: Promise<Record<string, string | string[] | undefined>>;
};
export type RouteContextInput = {
  params?: Promise<Record<string, string | string[] | undefined>>;
};
export function route(
  handler: (r: Request, context: RouteContext) => Promise<Response>,
): {
  (request: Request, context?: RouteContextInput): Promise<Response>;
  (request: Request, context: RouteContext): Promise<Response>;
} {
  return async (request: Request, context?: RouteContextInput) => {
    try {
      return await handler(
        request,
        { params: context?.params ?? Promise.resolve({}) },
      );
    } catch (e) {
      if (e instanceof HttpError)
        return Response.json(
          { error: e.code, message: e.message },
          { status: e.status, headers: { "Cache-Control": "no-store" } },
        );
      return Response.json(
        {
          error: "UNAVAILABLE",
          message:
            "Não foi possível concluir. Tente consultar o estado novamente.",
        },
        { status: 503, headers: { "Cache-Control": "no-store" } },
      );
    }
  };
}
export function response(body: unknown) {
  return Response.json(body, { headers: { "Cache-Control": "no-store" } });
}
