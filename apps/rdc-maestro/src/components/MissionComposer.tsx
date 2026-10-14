import { useState } from "react";
import type { KeyboardEvent } from "react";

import ApprovePlanButton from "./ApprovePlanButton";
import type { PlanExecutionReceipt } from "../lib/maestro";

interface MissionComposerProps {
  busy: boolean;
  missionStatus: string;
  hasConsensus: boolean;
  sendingPlan: boolean;
  receipt: PlanExecutionReceipt | null;
  onSubmit: (prompt: string) => void;
  onApprove: () => void;
  onRollback?: (checkpointId: string) => void;
  rollingBack?: boolean;
}

export default function MissionComposer({
  busy,
  missionStatus,
  hasConsensus,
  sendingPlan,
  receipt,
  onSubmit,
  onApprove,
  onRollback,
  rollingBack,
}: MissionComposerProps) {
  const [prompt, setPrompt] = useState("");

  const submit = () => {
    const trimmed = prompt.trim();
    if (!trimmed || busy) {
      return;
    }
    onSubmit(trimmed);
    setPrompt("");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      submit();
    }
  };

  return (
    <footer className="border-t border-rdc-border bg-rdc-panel px-6 py-4">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3">
        <div className="flex items-end gap-3">
          <textarea
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={handleKeyDown}
            rows={3}
            spellCheck={false}
            placeholder="Descreva o objetivo da missão. Ex.: reiniciar o serviço de spooler em 12 máquinas sem derrubar as sessões RDP e propor um plano passo a passo."
            className="selectable flex-1 resize-none rounded-lg border border-rdc-border bg-[#07090e] px-3 py-2.5 font-mono text-[0.76rem] leading-relaxed text-slate-200 outline-none placeholder:text-slate-600 focus:border-cyan-500/60 scroll-slim"
          />
          <button
            type="button"
            onClick={submit}
            disabled={busy || prompt.trim().length === 0}
            className="h-[46px] shrink-0 rounded-lg bg-cyan-500 px-5 text-[0.78rem] font-semibold text-slate-950 transition-colors hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
          >
            {busy ? "Debatendo…" : "Enviar"}
          </button>
        </div>

        <div className="flex items-center justify-between gap-4">
          <p className="text-[0.66rem] text-slate-500">
            Ctrl+Enter envia · <span className="text-slate-400">{missionStatus}</span>
          </p>
          <ApprovePlanButton
            visible={hasConsensus}
            sending={sendingPlan}
            receipt={receipt}
            disabled={busy}
            onApprove={onApprove}
            onRollback={onRollback}
            rollingBack={rollingBack}
          />
        </div>
      </div>
    </footer>
  );
}
