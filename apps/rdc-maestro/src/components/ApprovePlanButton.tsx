import type { PlanExecutionReceipt } from "../lib/maestro";

interface ApprovePlanButtonProps {
  visible: boolean;
  sending: boolean;
  receipt: PlanExecutionReceipt | null;
  disabled: boolean;
  onApprove: () => void;
}

/// Resume o recibo JSON do daemon (`rdc.maestro.ack.v1`) para leitura do operador.
function ackSummary(response: string | null): string {
  if (!response) {
    return "o daemon não devolveu ack nesta conexão";
  }
  try {
    const ack = JSON.parse(response) as {
      status?: string;
      steps_executed?: number;
      steps_total?: number;
      failure?: { error?: { code?: string; message?: string } };
    };
    const status = ack.status ?? "desconhecido";
    const failure = ack.failure?.error;
    if (failure) {
      return `${status} · ${failure.code ?? "erro"}: ${failure.message ?? "sem detalhe"}`;
    }
    return `${status} · ${ack.steps_executed ?? 0}/${ack.steps_total ?? 0} passo(s) executados`;
  } catch {
    return response.slice(0, 120);
  }
}

export default function ApprovePlanButton({
  visible,
  sending,
  receipt,
  disabled,
  onApprove,
}: ApprovePlanButtonProps) {
  if (!visible) {
    return (
      <div className="flex items-center gap-2 text-[0.68rem] text-slate-500">
        <span className="h-1.5 w-1.5 rounded-full bg-slate-600" />
        Aguardando a síntese final para liberar a aplicação física.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
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

      {receipt ? (
        <p className="font-mono text-[0.62rem] text-emerald-300/80">
          {receipt.endpoint} · {receipt.bytes_written} bytes · {ackSummary(receipt.response)}
        </p>
      ) : null}
    </div>
  );
}
