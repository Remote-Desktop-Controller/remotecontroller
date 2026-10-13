import { useEffect, useRef } from "react";

import { PROVIDER_LABELS } from "../lib/maestro";

export interface FeedItem {
  id: string;
  kind: "user" | "agent" | "consensus" | "system" | "dispatch" | "error";
  provider?: string;
  title?: string;
  text: string;
  ts: number;
  streaming?: boolean;
}

interface BlackboardFeedProps {
  items: FeedItem[];
  planText: string;
  onPlanChange: (value: string) => void;
  hasConsensus: boolean;
}

const KIND_STYLE: Record<FeedItem["kind"], { label: string; chip: string; border: string }> = {
  user: {
    label: "Operador",
    chip: "bg-cyan-500/15 text-cyan-300 border-cyan-500/30",
    border: "border-cyan-500/30",
  },
  agent: {
    label: "Rodada 1",
    chip: "bg-slate-500/15 text-slate-300 border-slate-500/30",
    border: "border-rdc-border",
  },
  consensus: {
    label: "Síntese final",
    chip: "bg-emerald-500/15 text-emerald-300 border-emerald-500/30",
    border: "border-emerald-500/40",
  },
  system: {
    label: "Sistema",
    chip: "bg-slate-500/10 text-slate-400 border-slate-600/30",
    border: "border-rdc-border",
  },
  dispatch: {
    label: "Daemon",
    chip: "bg-blue-500/15 text-blue-300 border-blue-500/30",
    border: "border-blue-500/40",
  },
  error: {
    label: "Erro",
    chip: "bg-rose-500/15 text-rose-300 border-rose-500/30",
    border: "border-rose-500/40",
  },
};

export default function BlackboardFeed({
  items,
  planText,
  onPlanChange,
  hasConsensus,
}: BlackboardFeedProps) {
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [items]);

  return (
    <div className="flex-1 overflow-y-auto px-6 py-5 scroll-slim">
      {items.length === 0 ? (
        <div className="mx-auto mt-16 max-w-lg rounded-xl border border-dashed border-rdc-border px-6 py-10 text-center">
          <p className="text-sm font-medium text-slate-300">Blackboard vazio</p>
          <p className="mt-2 text-xs leading-relaxed text-slate-500">
            Autentique ChatGPT, Claude e Gemini na barra lateral e descreva o objetivo no rodapé.
            O prompt é injetado simultaneamente nas três sessões ocultas e as respostas aparecem
            aqui conforme chegam.
          </p>
        </div>
      ) : null}

      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3">
        {items.map((item) => {
          const style = KIND_STYLE[item.kind];
          const heading =
            item.title ??
            (item.provider ? (PROVIDER_LABELS[item.provider] ?? item.provider) : style.label);

          return (
            <article
              key={item.id}
              className={`rounded-xl border ${style.border} bg-rdc-card/70 px-4 py-3`}
            >
              <header className="mb-2 flex items-center gap-2">
                <span
                  className={`rounded border px-1.5 py-0.5 text-[0.6rem] font-semibold uppercase tracking-[0.08em] ${style.chip}`}
                >
                  {style.label}
                </span>
                <span className="text-[0.72rem] font-medium text-slate-300">{heading}</span>
                {item.streaming ? (
                  <span className="flex items-center gap-1 text-[0.62rem] text-amber-300">
                    <span className="h-1.5 w-1.5 rounded-full bg-amber-400 dot-thinking" />
                    streaming
                  </span>
                ) : null}
                <span className="ml-auto font-mono text-[0.6rem] text-slate-600">
                  {new Date(item.ts).toLocaleTimeString("pt-BR", { hour12: false })}
                </span>
              </header>

              {item.kind === "consensus" && hasConsensus ? (
                <div className="space-y-3">
                  <textarea
                    value={planText}
                    onChange={(event) => onPlanChange(event.target.value)}
                    spellCheck={false}
                    className="selectable h-64 w-full resize-y rounded-lg border border-rdc-border bg-[#07090e] p-3 font-mono text-[0.72rem] leading-relaxed text-slate-200 outline-none focus:border-emerald-500/60 scroll-slim"
                  />
                  <p className="text-[0.62rem] text-slate-500">
                    Edite o texto acima para definir exatamente a ordem física que será enviada ao
                    daemon.
                  </p>
                </div>
              ) : (
                <pre className="selectable max-h-64 overflow-y-auto whitespace-pre-wrap break-words font-mono text-[0.72rem] leading-relaxed text-slate-300 scroll-slim">
                  {item.text}
                </pre>
              )}
            </article>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
}
