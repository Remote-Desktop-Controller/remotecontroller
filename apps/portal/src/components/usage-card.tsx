"use client";
import Link from "next/link";

export type DailyUsage = {
  plan: "FREE" | "PREMIUM" | "ADMIN";
  remaining_seconds: number;
  used_seconds?: number;
  resets_at: number;
  expires_at: number | null;
};
const brasilia = (seconds: number) => new Intl.DateTimeFormat("pt-BR", {
  timeZone:"America/Sao_Paulo", dateStyle:"short", timeStyle:"short",
}).format(new Date(seconds * 1000));
export function UsageCard({usage}:{usage:DailyUsage}) {
  const remaining = Math.min(1800, Math.max(0, Math.floor(usage.remaining_seconds)));
  const used = 1800 - remaining;
  return <section className="surface-card subscription-card" aria-label="Plano e consumo diário">
    <p className="eyebrow">PLANO ATUAL</p>
    <h2>{usage.plan === "FREE" ? "Gratuito" : usage.plan === "ADMIN" ? "Licença administrativa" : "Premium"}</h2>
    {usage.plan === "FREE" ? <>
      <p><strong>{Math.floor(remaining/60)} min {remaining%60} s</strong> gratuitos restantes hoje</p>
      <progress aria-label="Tempo gratuito utilizado hoje" max={1800} value={used} style={{width:"100%",accentColor:"#168754"}}/>
      {remaining === 0 && <p role="status"><strong>Limite gratuito de hoje atingido.</strong> Novas operações ficam bloqueadas até a renovação ou ativação do Premium.</p>}
      <p>Próxima renovação: <strong>{brasilia(usage.resets_at)} (Brasília)</strong>. Os minutos não acumulam.</p>
      <p>Login e computador online não gastam minutos. O saldo inclui reservas para operações em execução e é atualizado pelo servidor.</p>
      <Link className="button button-green" href="/checkout">Assinar Premium — R$ 9,90 / 30 dias</Link>
    </> : <>
      <p>Sem limite diário de minutos. Sua franquia gratuita fica preservada.</p>
      {usage.expires_at && <p>Válido até <strong>{brasilia(usage.expires_at)} (Brasília)</strong>.</p>}
      {usage.plan === "PREMIUM" && <p>Após a expiração, sua conta retorna automaticamente ao Gratuito.</p>}
    </>}
  </section>;
}
