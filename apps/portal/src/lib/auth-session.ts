export type SessionUser = {
  id: string;
  email: string;
  emailVerified: true;
  confirmedAt: string;
};

// Preserve encoded callback query parameters; reject unsafe path forms only.
export function safeNext(value: string | null): string {
  const fallback = "/dashboard";
  if (!value || !value.startsWith("/") || value.startsWith("//") ||
      value.includes("\\") || /[\x00-\x20\x7f]/.test(value)) return fallback;
  try {
    const target = new URL(value, "https://portal.invalid");
    if (target.origin !== "https://portal.invalid" || /%(?:2f|5c|00|0a|0d)/i.test(target.pathname)) return fallback;
    return target.pathname + target.search + target.hash;
  } catch {
    return fallback;
  }
}
