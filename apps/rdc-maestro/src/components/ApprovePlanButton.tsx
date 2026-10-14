import type { PlanExecutionReceipt } from "../lib/maestro";

interface ApprovePlanButtonProps {
  visible: boolean;
  sending: boolean;
  receipt: PlanExecutionReceipt | null;
  disabled: boolean;
  onApprove: () => void;
  onRollback?: (checkpointId: string) => void;
  rollingBack?: boolean;
}

/// Resume o recibo JSON do daemon (`rdc.maestro.ack.v1`) para leitura do operador.
function ackSummary(response: string | null): {
  text: string;
  checkpointId: string | null;
  isCompleted: boolean;
} {
  if (!response) {
    return { text: "o daemon não devolveu ack nesta conexão", checkpointId: null, isCompleted: false };
  }
  try {
    const ack = JSON.parse(response) as {
      status?: string;
      checkpoint_id?: string;
      steps_executed?: number;
      steps_total?: number;
      failure?: { error?: { code?: string; message?: string } };
      steps?: Array<{ checkpoint_id?: string; status?: string }>;
      restored_files?: string[];
    };
    const status = ack.status ?? "desconhecido";
    const failure = ack.failure?.error;
    const isCompleted = status === "completed";
    let checkpointId = ack.checkpoint_id ?? null;
    if (!checkpointId && Array.isArray(ack.steps)) {
      const stepWithCp = ack.steps.find((s) => s.checkpoint_id);
      if (stepWithCp) {
        checkpointId = stepWithCp.checkpoint_id ?? null;
      }
    }
    if (ack.restored_files && ack.restored_files.length > 0) {
      return {
        text: `${status} · revertido(s) ${ack.restored_files.length} arquivo(s): ${ack.restored_files.join(", ")}`,
        checkpointId,
        isCompleted,
      };
    }
    if (failure) {
      return {
        text: `${status} · ${failure.code ?? "erro"}: ${failure.message ?? "sem detalhe"}`,
        checkpointId,
        isCompleted,
      };
    }
    return {
      text: `${status} · ${ack.steps_executed ?? 0}/${ack.steps_total ?? 0} passo(s) executados`,
      checkpointId,
      isCompleted,
    };
  } catch {
    return { text: response.slice(0, 120), checkpointId: null, isCompleted: false };
  }
}

export default function ApprovePlanButton({
  visible,
  sending,
  receipt,
  disabled,
  onApprove,
  onRollback,
  rollingBack,
}: ApprovePlanButtonProps) {
  if (!visible) {
    return (
      <div className="flex items-center gap-2 text-[0.68rem] text-slate-500">
        <span className="h-1.5 w-1.5 rounded-full bg-slate-600" />
        Aguardando a síntese final para liberar a aplicação física.
      </div>
    );
  }

  const { text: summaryText, checkpointId, isCompleted } = ackSummary(receipt?.response ?? null);

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onApprove}
          disabled={sending || disabled}
          className={`neon-approve inline-flex items-center justify-center gap-2 rounded-lg border border-emerald-300/60 bg-emerald-500 px-5 py-2.5 text-[0.8rem] font-bold uppercase tracking-[0.08em] text-emerald-950 transition-all hover:bg-emerald-400 disabled:cursor-not-allowed disabled:opacity-60 ${
            sending ? "" : "hover:scale-[1.02]"
          }`}
        >
          {sending ? "Aplicando no PC…" : "Aprovar e aplicar no PC"}
        </button>

        {isCompleted && checkpointId && onRollback ? (
          <button
            type="button"
            onClick={() => onRollback(checkpointId)}
            disabled={rollingBack || sending}
            className="inline-flex items-center justify-center gap-1.5 rounded-lg border border-amber-500/50 bg-amber-500/20 px-3 py-2 text-[0.72rem] font-bold text-amber-300 transition-all hover:bg-amber-500/30 disabled:opacity-50"
            title="Reverter alterações físicas usando o checkpoint gravado"
          >
            {rollingBack ? "Revertendo…" : "↩ Desfazer no PC (Rollback Checkpoint)"}
          </button>
        ) : null}
      </div>

      {receipt ? (
        <p className="font-mono text-[0.62rem] text-emerald-300/80">
          {receipt.endpoint} · {receipt.bytes_written} bytes · {summaryText}
          {checkpointId ? ` · checkpoint: ${checkpointId}` : ""}
        </p>
      ) : null}
    </div>
  );
}

