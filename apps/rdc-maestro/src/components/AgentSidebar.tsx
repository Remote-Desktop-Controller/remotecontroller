import { PROVIDER_IDS, PROVIDER_LABELS } from "../lib/maestro";
import type { ProviderId, ProviderStatus } from "../lib/maestro";

interface AgentSidebarProps {
  providers: ProviderStatus[];
  thinking: Set<string>;
  activeMission: string | null;
  onLogin: (provider: ProviderId) => void;
  onShow: (provider: ProviderId) => void;
  onHide: (provider: ProviderId) => void;
  onConfirmLogin: (provider: ProviderId) => void;
}

type DotTone = "connected" | "thinking" | "authenticating" | "offline";

function dotTone(status: ProviderStatus | undefined, isThinking: boolean): DotTone {
  if (isThinking) {
    return "thinking";
  }
  if (!status || status.state === "disconnected") {
    return "offline";
  }
  return status.state === "connected" ? "connected" : "authenticating";
}

const DOT_CLASS: Record<DotTone, string> = {
  connected: "bg-emerald-400 shadow-[0_0_10px_rgba(16,185,129,0.9)]",
  thinking: "bg-amber-400 shadow-[0_0_10px_rgba(245,158,11,0.9)] dot-thinking",
  authenticating: "bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.6)]",
  offline: "bg-slate-600",
};

const TONE_LABEL: Record<DotTone, string> = {
  connected: "Conectado",
  thinking: "Pensando…",
  authenticating: "Autenticando",
  offline: "Desconectado",
};

export default function AgentSidebar({
  providers,
  thinking,
  activeMission,
  onLogin,
  onShow,
  onHide,
  onConfirmLogin,
}: AgentSidebarProps) {
  const byProvider = new Map(providers.map((status) => [status.id, status]));

  return (
    <aside className="flex w-[272px] shrink-0 flex-col border-r border-rdc-border bg-rdc-panel">
      <div className="flex items-center gap-3 border-b border-rdc-border px-4 py-4">
        <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-gradient-to-br from-cyan-400 to-blue-600 text-[0.7rem] font-bold text-slate-950">
          RDC
        </div>
        <div className="leading-tight">
          <p className="text-sm font-semibold tracking-wide">MAESTRO</p>
          <p className="text-[0.68rem] text-slate-500">Orquestrador Multi-Agente</p>
        </div>
      </div>

      <div className="px-4 pt-4">
        <p className="text-[0.65rem] font-semibold uppercase tracking-[0.12em] text-slate-500">
          Agentes
        </p>
      </div>

      <div className="flex flex-1 flex-col gap-2 overflow-y-auto px-3 py-3 scroll-slim">
        {PROVIDER_IDS.map((id) => {
          const status = byProvider.get(id);
          const isThinking = thinking.has(id);
          const tone = dotTone(status, isThinking);

          return (
            <div
              key={id}
              className="rounded-lg border border-rdc-border bg-rdc-card px-3 py-2.5 transition-colors hover:border-slate-600"
            >
              <div className="flex items-center gap-2.5">
                <span className={`h-2.5 w-2.5 shrink-0 rounded-full ${DOT_CLASS[tone]}`} />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[0.82rem] font-medium">
                    {PROVIDER_LABELS[id] ?? id}
                  </p>
                  <p className="truncate text-[0.66rem] text-slate-500">{TONE_LABEL[tone]}</p>
                </div>
              </div>

              <div className="mt-2 flex gap-1.5">
                <button
                  type="button"
                  onClick={() => onLogin(id)}
                  className="flex-1 rounded border border-rdc-border px-2 py-1 text-[0.66rem] text-slate-300 transition-colors hover:border-cyan-500/60 hover:text-cyan-300"
                >
                  Login
                </button>
                <button
                  type="button"
                  onClick={() => onShow(id)}
                  title="Exibir a janela da sessão"
                  className="rounded border border-rdc-border px-2 py-1 text-[0.66rem] text-slate-300 transition-colors hover:border-slate-500 hover:text-slate-100"
                >
                  Ver
                </button>
                <button
                  type="button"
                  onClick={() => onHide(id)}
                  title="Ocultar a janela mantendo a sessão viva"
                  className="rounded border border-rdc-border px-2 py-1 text-[0.66rem] text-slate-300 transition-colors hover:border-slate-500 hover:text-slate-100"
                >
                  Ocultar
                </button>
              </div>

              {status?.state === "authenticating" ? (
                <button
                  type="button"
                  onClick={() => onConfirmLogin(id)}
                  title="Confirmar autenticação manual e ocultar janela em background"
                  className="mt-1.5 w-full rounded border border-emerald-500/60 bg-emerald-500/15 px-2 py-1 text-[0.68rem] font-medium text-emerald-300 transition-colors hover:border-emerald-400 hover:bg-emerald-500/25 hover:text-emerald-200"
                >
                  Confirmar Login
                </button>
              ) : null}

              {status?.detail ? (
                <p className="mt-2 line-clamp-2 text-[0.62rem] leading-snug text-slate-500">
                  {status.detail}
                </p>
              ) : null}
            </div>
          );
        })}
      </div>

      <div className="border-t border-rdc-border px-4 py-3">
        <p className="text-[0.62rem] uppercase tracking-[0.12em] text-slate-500">Blackboard</p>
        <p className="mt-1 font-mono text-[0.66rem] text-slate-400">
          {activeMission ? `missão ${activeMission}` : "nenhuma missão ativa"}
        </p>
        <p className="mt-2 text-[0.6rem] leading-snug text-slate-600">
          Conectado = verde · Pensando = amarelo · Sessões ficam ocultas após o login.
        </p>
      </div>
    </aside>
  );
}
