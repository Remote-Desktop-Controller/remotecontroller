import { useEffect, useRef } from "react";

import { PROVIDER_LABELS } from "../lib/maestro";
import type { ContextLog } from "../lib/maestro";

export interface FeedItem {
  id: string;
  kind: "user" | "agent" | "consensus" | "system" | "dispatch" | "error" | "autoread";
  provider?: string;
  title?: string;
  text: string;
  ts: number;
  streaming?: boolean;
  turn?: number;
  context_log?: ContextLog | null;
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
    label: "Agente",
    chip: "bg-slate-500/15 text-slate-300 border-slate-500/30",
    border: "border-rdc-border",
  },
  autoread: {
    label: "Leitura Autônoma",
    chip: "bg-purple-500/15 text-purple-300 border-purple-500/30",
    border: "border-purple-500/40",
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

interface SearchReplaceBlock {
  file: string;
  search: string;
  replace: string;
}

function extractSearchReplaceBlocks(text: string): { blocks: SearchReplaceBlock[]; hasPatches: boolean } {
  const regex = /<<<<<<< SEARCH\s*:?\s*([^\r\n]+)[\r\n]+([\s\S]*?)=======[\r\n]+([\s\S]*?)>>>>>>> REPLACE/g;
  const blocks: SearchReplaceBlock[] = [];
  let match;
  while ((match = regex.exec(text)) !== null) {
    const rawPath = match[1].trim().replace(/^[`'"]+|[`'"]+$/g, "").replace(/^file:\s*/i, "");
    blocks.push({
      file: rawPath,
      search: match[2],
      replace: match[3],
    });
  }
  return { blocks, hasPatches: blocks.length > 0 };
}

function SearchReplaceRenderer({ text }: { text: string }) {
  const { blocks, hasPatches } = extractSearchReplaceBlocks(text);
  if (!hasPatches) {
    return (
      <pre className="selectable max-h-64 overflow-y-auto whitespace-pre-wrap break-words font-mono text-[0.72rem] leading-relaxed text-slate-300 scroll-slim">
        {text}
      </pre>
    );
  }

  return (
    <div className="space-y-3 font-mono text-[0.72rem]">
      {blocks.map((block, idx) => (
        <div key={idx} className="rounded-lg border border-slate-700/60 bg-[#06080d] p-3">
          <div className="mb-2 flex items-center justify-between border-b border-slate-800 pb-1.5">
            <span className="flex items-center gap-1.5 font-semibold text-cyan-300">
              <span className="h-1.5 w-1.5 rounded-full bg-cyan-400" />
              {block.file}
            </span>
            <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[0.6rem] text-slate-400">
              SEARCH/REPLACE
            </span>
          </div>

          <div className="space-y-1.5">
            {block.search.trim() ? (
              <div className="rounded border-l-2 border-rose-500/80 bg-rose-500/10 px-2 py-1 text-rose-300">
                <div className="mb-0.5 text-[0.6rem] font-bold uppercase tracking-wide text-rose-400/80">
                  - Busca (Original)
                </div>
                <pre className="whitespace-pre-wrap break-words leading-tight">{block.search}</pre>
              </div>
            ) : null}

            <div className="rounded border-l-2 border-emerald-500/80 bg-emerald-500/10 px-2 py-1 text-emerald-300">
              <div className="mb-0.5 text-[0.6rem] font-bold uppercase tracking-wide text-emerald-400/80">
                + Substituição (Patch)
              </div>
              <pre className="whitespace-pre-wrap break-words leading-tight">{block.replace}</pre>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function ComplianceGateCard({ contextLog }: { contextLog: ContextLog }) {
  return (
    <div className="my-2 rounded-lg border border-emerald-500/40 bg-emerald-500/10 p-3 text-[0.72rem]">
      <div className="mb-2 flex items-center justify-between border-b border-emerald-500/20 pb-1.5">
        <span className="flex items-center gap-1.5 font-bold uppercase tracking-wider text-emerald-300">
          <span className="h-2 w-2 rounded-full bg-emerald-400 animate-pulse" />
          Gate de Conformidade (Context Log)
        </span>
        <span className="rounded bg-emerald-500/20 px-2 py-0.5 text-[0.62rem] font-semibold text-emerald-200">
          ✓ Atestado pelo Maestro
        </span>
      </div>
      <div className="grid grid-cols-1 gap-2 text-slate-200">
        <div>
          <span className="text-[0.62rem] font-bold uppercase text-slate-400">Objetivo: </span>
          <span className="text-emerald-100">{contextLog.intent}</span>
        </div>
        {contextLog.files_inspected && contextLog.files_inspected.length > 0 ? (
          <div>
            <span className="text-[0.62rem] font-bold uppercase text-slate-400">Arquivos Inspecionados: </span>
            <div className="mt-1 flex flex-wrap gap-1">
              {contextLog.files_inspected.map((file, i) => (
                <span
                  key={i}
                  className="rounded border border-cyan-500/40 bg-cyan-500/15 px-1.5 py-0.5 font-mono text-[0.65rem] text-cyan-300"
                >
                  {file}
                </span>
              ))}
            </div>
          </div>
        ) : null}
        <div>
          <span className="text-[0.62rem] font-bold uppercase text-slate-400">Plano de Verificação: </span>
          <span className="text-slate-300">{contextLog.verification_plan}</span>
        </div>
      </div>
    </div>
  );
}

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
            Autentique os agentes na barra lateral e descreva o objetivo no rodapé.
            O prompt é injetado simultaneamente nas sessões ocultas e o loop autônomo ReAct
            inspeciona arquivos locais antes de propor o plano cirúrgico.
          </p>
        </div>
      ) : null}

      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3">
        {items.map((item) => {
          const style = KIND_STYLE[item.kind] ?? KIND_STYLE.system;
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
                {item.turn ? (
                  <span className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[0.62rem] text-slate-400">
                    Turno {item.turn}/3
                  </span>
                ) : null}
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

              {item.context_log ? (
                <ComplianceGateCard contextLog={item.context_log} />
              ) : null}

              {item.kind === "consensus" && hasConsensus ? (
                <div className="space-y-3">
                  <SearchReplaceRenderer text={planText} />
                  <textarea
                    value={planText}
                    onChange={(event) => onPlanChange(event.target.value)}
                    spellCheck={false}
                    className="selectable h-48 w-full resize-y rounded-lg border border-rdc-border bg-[#07090e] p-3 font-mono text-[0.72rem] leading-relaxed text-slate-200 outline-none focus:border-emerald-500/60 scroll-slim"
                  />
                  <p className="text-[0.62rem] text-slate-500">
                    Edite o texto acima para ajustar a ordem física enviada ao daemon (SEARCH/REPLACE ou passos JSON).
                  </p>
                </div>
              ) : (
                <SearchReplaceRenderer text={item.text} />
              )}
            </article>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
}

