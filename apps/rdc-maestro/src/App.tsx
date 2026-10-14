import { useCallback, useEffect, useMemo, useState } from "react";

import AgentSidebar from "./components/AgentSidebar";
import BlackboardFeed from "./components/BlackboardFeed";
import type { FeedItem } from "./components/BlackboardFeed";
import MissionComposer from "./components/MissionComposer";
import {
  MAESTRO_EVENTS,
  PROVIDER_IDS,
  PROVIDER_LABELS,
  formatError,
  maestroApi,
  useTauriEvent,
} from "./lib/maestro";
import type {
  AutoReadStartedEvent,
  DebateOutcome,
  DebateProgress,
  MaestroLog,
  PlanExecutionReceipt,
  ProviderId,
  ProviderStatus,
} from "./lib/maestro";

let feedSequence = 0;
const nextId = (): string => {
  feedSequence += 1;
  return `feed-${feedSequence}`;
};

/// Sessões antigas podem emitir o nome de exibição em vez do id.
function providerKey(raw: string): string {
  const normalized = raw.toLowerCase();
  const found = PROVIDER_IDS.find(
    (id) => id === normalized || PROVIDER_LABELS[id].toLowerCase() === normalized
  );
  return found ?? normalized;
}

export default function App() {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [roundActive, setRoundActive] = useState(false);
  const [replied, setReplied] = useState<Set<string>>(() => new Set());
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [activeMission, setActiveMission] = useState<string | null>(null);
  const [missionStatus, setMissionStatus] = useState("Aguardando missão");
  const [consensus, setConsensus] = useState<DebateOutcome | null>(null);
  const [planText, setPlanText] = useState("");
  const [busy, setBusy] = useState(false);
  const [sendingPlan, setSendingPlan] = useState(false);
  const [rollingBack, setRollingBack] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [receipt, setReceipt] = useState<PlanExecutionReceipt | null>(null);

  const pushFeed = useCallback((item: Omit<FeedItem, "id" | "ts"> & { ts?: number }) => {
    setFeed((current) => [...current, { id: nextId(), ts: item.ts ?? Date.now(), ...item }]);
  }, []);

  // Pensando = sessão conectada que ainda não devolveu a resposta da rodada corrente.
  const thinking = useMemo(() => {
    const set = new Set<string>();
    if (!roundActive) {
      return set;
    }
    for (const status of providers) {
      if (status.state === "connected" && !replied.has(status.id)) {
        set.add(status.id);
      }
    }
    return set;
  }, [providers, replied, roundActive]);

  // Carga inicial: status das sessões e último consenso (reidratação após reload).
  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const [statuses, last] = await Promise.all([
          maestroApi.providerStatuses(),
          maestroApi.lastConsensus(),
        ]);
        if (cancelled) {
          return;
        }
        setProviders(statuses);
        if (last) {
          setConsensus(last);
          setPlanText(last.consensus);
          setMissionStatus(
            `Síntese anterior · ${last.coverage}/${PROVIDER_IDS.length} agentes · missão ${last.mission_id}`
          );
          pushFeed({
            kind: "consensus",
            title: `síntese anterior · ${last.mission_id}`,
            text: last.consensus,
            ts: last.finished_at,
          });
        }
      } catch (error) {
        pushFeed({ kind: "error", title: "IPC", text: formatError(error) });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [pushFeed]);

  useTauriEvent<ProviderStatus>(MAESTRO_EVENTS.providerStatus, (status) => {
    setProviders((current) =>
      [...current.filter((item) => item.id !== status.id), status].sort((a, b) =>
        a.id.localeCompare(b.id)
      )
    );
  });

  useTauriEvent<MaestroLog>(MAESTRO_EVENTS.log, (log) => {
    if (log.level === "info") {
      return; // o progresso detalhado já chega pelos eventos de debate
    }
    pushFeed({
      kind: log.level === "error" ? "error" : "system",
      title: log.level,
      text: log.message,
    });
  });

  useTauriEvent<DebateProgress>(MAESTRO_EVENTS.progress, (progress) => {
    setActiveMission(progress.mission_id);

    if (progress.stage === "prompt_injected") {
      setRoundActive(true);
      setMissionStatus(`Rodada 1 · ${progress.detail}`);
      pushFeed({ kind: "system", title: "Rodada 1", text: progress.detail });
      return;
    }

    if (progress.stage === "reply_received" && progress.provider) {
      const provider = providerKey(progress.provider);
      setReplied((current) => new Set(current).add(provider));
      setMissionStatus(`Rodada 1 · ${progress.received}/${progress.expected} respostas`);
      pushFeed({
        kind: "agent",
        provider,
        title: `${PROVIDER_LABELS[provider] ?? provider} · resposta consolidada`,
        text: "Resposta recebida pelo blackboard. O texto completo aparece na síntese final.",
      });
    }
  });

  useTauriEvent<DebateProgress>(MAESTRO_EVENTS.partial, (progress) => {
    if (!progress.provider) {
      return;
    }
    const provider = providerKey(progress.provider);
    setFeed((current) => {
      const index = current.findIndex(
        (item) => item.kind === "agent" && item.provider === provider && item.streaming
      );
      const entry: FeedItem = {
        id: index >= 0 ? current[index].id : nextId(),
        kind: "agent",
        provider,
        title: `${PROVIDER_LABELS[provider] ?? provider} · streaming parcial`,
        text: progress.detail,
        ts: progress.timestamp,
        streaming: true,
      };
      if (index < 0) {
        return [...current, entry];
      }
      const next = [...current];
      next[index] = entry;
      return next;
    });
  });

  useTauriEvent<DebateOutcome>(MAESTRO_EVENTS.consensus, (outcome) => {
    setConsensus(outcome);
    setPlanText(outcome.consensus);
    setActiveMission(outcome.mission_id);
    setRoundActive(false);
    const turnSuffix = outcome.turns_count ? ` · ${outcome.turns_count} turnos ReAct` : "";
    setMissionStatus(
      `Síntese final · ${outcome.coverage}/${PROVIDER_IDS.length} agentes · confiança ${(outcome.confidence * 100).toFixed(0)}%${turnSuffix}`
    );

    setFeed((current) => [
      ...current.filter((item) => !item.streaming),
      {
        id: nextId(),
        kind: "agent",
        title: `Respostas completas · ${outcome.turns_count ? `Turno ${outcome.turns_count}` : "Rodada 1"}`,
        turn: outcome.turns_count,
        text: outcome.replies
          .map(
            (reply) =>
              `### ${PROVIDER_LABELS[reply.provider] ?? reply.provider} (${reply.latency_ms} ms)\n${reply.text}`
          )
          .join("\n\n"),
        ts: outcome.finished_at,
      },
      {
        id: nextId(),
        kind: "consensus",
        title: `síntese final · ${outcome.mission_id}${outcome.turns_count ? ` (${outcome.turns_count} turnos)` : ""}`,
        turn: outcome.turns_count,
        text: outcome.consensus,
        context_log: outcome.context_log,
        ts: outcome.finished_at,
      },
    ]);
  });

  useTauriEvent<AutoReadStartedEvent>(MAESTRO_EVENTS.autoReadStarted, (autoRead) => {
    pushFeed({
      kind: "autoread",
      title: `Leitura Autônoma · Turno ${autoRead.turn}/3`,
      turn: autoRead.turn,
      text: `Inspecionando arquivos do workspace no PC para realimentar os agentes:\n${autoRead.steps
        .map((s) => `- ${s.tool}: ${JSON.stringify(s.input ?? {})}`)
        .join("\n")}`,
    });
  });

  useTauriEvent<string>(MAESTRO_EVENTS.sessionsReset, (header) => {
    setConsensus(null);
    setPlanText("");
    setReceipt(null);
    setMissionStatus("Sessões limpas · Novo chat iniciado");
    pushFeed({
      kind: "system",
      title: "Compact & Reset",
      text: `Histórico compactado com sucesso no disco. Novo chat limpo iniciado em todos os provedores.\n\n${header}`,
    });
  });

  useTauriEvent<PlanExecutionReceipt>(MAESTRO_EVENTS.rollbackCompleted, (dispatched) => {
    setReceipt(dispatched);
    pushFeed({
      kind: "system",
      title: "Rollback Concluído",
      text: `Checkpoint revertido com sucesso no PC (${dispatched.endpoint}). Resposta: ${dispatched.response ?? "OK"}`,
    });
  });

  useTauriEvent<string>(MAESTRO_EVENTS.failed, (message) => {
    setRoundActive(false);
    setMissionStatus("Falha no ciclo de debate");
    pushFeed({ kind: "error", title: "debate", text: message });
  });

  useTauriEvent<PlanExecutionReceipt>(MAESTRO_EVENTS.dispatched, (dispatched) => {
    setReceipt(dispatched);
    pushFeed({
      kind: "dispatch",
      title: dispatched.endpoint,
      text: `Ordem despachada (${dispatched.bytes_written} bytes, encoding ${dispatched.encoding}). ${
        dispatched.acknowledged
          ? `Ack do daemon: ${dispatched.response ?? ""}`
          : "O daemon não devolveu ack nesta conexão."
      }`,
    });
  });

  const handleSubmit = useCallback(
    async (prompt: string) => {
      setBusy(true);
      setConsensus(null);
      setPlanText("");
      setReceipt(null);
      setReplied(new Set());
      setRoundActive(true);
      setMissionStatus("Injetando prompt nas sessões ocultas…");
      pushFeed({ kind: "user", text: prompt });
      pushFeed({
        kind: "system",
        title: "maestro_start_mission",
        text: `Prompt despachado para ${Object.values(PROVIDER_LABELS).join(", ")} em paralelo.`,
      });

      try {
        const outcome = await maestroApi.startMission(prompt);
        setMissionStatus(
          `Concluído · ${outcome.coverage}/${PROVIDER_IDS.length} agentes · ${outcome.elapsed_ms} ms · ${
            outcome.quorum_met ? "quórum atingido" : "quórum não atingido"
          }`
        );
      } catch (error) {
        setRoundActive(false);
        setMissionStatus("Falha ao iniciar a missão");
        pushFeed({ kind: "error", title: "maestro_start_mission", text: formatError(error) });
      } finally {
        setBusy(false);
      }
    },
    [pushFeed]
  );

  const handleApprove = useCallback(async () => {
    const payload = planText.trim();
    if (!payload) {
      pushFeed({
        kind: "error",
        title: "plano vazio",
        text: "Gere ou escreva um plano antes de aplicar no PC.",
      });
      return;
    }

    setSendingPlan(true);
    pushFeed({
      kind: "system",
      title: "maestro_execute_plan",
      text: `Enviando ordem aprovada (${payload.length} caracteres) ao Named Pipe do daemon…`,
    });

    try {
      const dispatched = await maestroApi.executePlan(payload);
      setReceipt(dispatched);
    } catch (error) {
      pushFeed({ kind: "error", title: "maestro_execute_plan", text: formatError(error) });
    } finally {
      setSendingPlan(false);
    }
  }, [planText, pushFeed]);

  const handleLogin = useCallback(
    async (provider: ProviderId) => {
      try {
        const status = await maestroApi.openLogin(provider);
        setProviders((current) => [...current.filter((item) => item.id !== status.id), status]);
        pushFeed({
          kind: "system",
          title: "open_provider_login",
          text: `Janela de ${PROVIDER_LABELS[provider]} aberta para autenticação.`,
        });
      } catch (error) {
        pushFeed({ kind: "error", title: provider, text: formatError(error) });
      }
    },
    [pushFeed]
  );

  const handleShow = useCallback(
    async (provider: ProviderId) => {
      try {
        const status = await maestroApi.showProvider(provider);
        setProviders((current) => [...current.filter((item) => item.id !== status.id), status]);
      } catch (error) {
        pushFeed({ kind: "error", title: provider, text: formatError(error) });
      }
    },
    [pushFeed]
  );

  const handleConfirmLogin = useCallback(
    async (provider: ProviderId) => {
      try {
        const status = await maestroApi.setProviderConnected(provider);
        setProviders((current) => [...current.filter((item) => item.id !== status.id), status]);
        pushFeed({
          kind: "system",
          title: "set_provider_connected",
          text: `Sessão de ${PROVIDER_LABELS[provider]} confirmada como conectada e ocultada em background.`,
        });
      } catch (error) {
        pushFeed({ kind: "error", title: provider, text: formatError(error) });
      }
    },
    [pushFeed]
  );

  const handleHide = useCallback(
    async (provider: ProviderId) => {
      try {
        const currentStatus = providers.find((item) => item.id === provider);
        const status =
          currentStatus?.state === "authenticating"
            ? await maestroApi.setProviderConnected(provider)
            : await maestroApi.hideProvider(provider);
        setProviders((current) => [...current.filter((item) => item.id !== status.id), status]);
      } catch (error) {
        pushFeed({ kind: "error", title: provider, text: formatError(error) });
      }
    },
    [providers, pushFeed]
  );

  const handleRollback = useCallback(
    async (checkpointId: string) => {
      setRollingBack(true);
      pushFeed({
        kind: "system",
        title: "maestro_rollback_checkpoint",
        text: `Solicitando reversão do checkpoint ${checkpointId} no daemon…`,
      });

      try {
        const dispatched = await maestroApi.rollbackCheckpoint(checkpointId);
        setReceipt(dispatched);
      } catch (error) {
        pushFeed({ kind: "error", title: "maestro_rollback_checkpoint", text: formatError(error) });
      } finally {
        setRollingBack(false);
      }
    },
    [pushFeed]
  );

  const handleCompactAndReset = useCallback(async () => {
    setCompacting(true);
    pushFeed({
      kind: "system",
      title: "maestro_compact_and_reset_sessions",
      text: "Compactando histórico das missões e reiniciando chats nos navegadores…",
    });

    try {
      const header = await maestroApi.compactAndResetSessions();
      setConsensus(null);
      setPlanText("");
      setReceipt(null);
      setMissionStatus("Sessões limpas · Novo chat iniciado");
      pushFeed({
        kind: "system",
        title: "Compact & Reset Concluído",
        text: `Histórico compactado com sucesso. Cabeçalho mantido:\n\n${header}`,
      });
    } catch (error) {
      pushFeed({ kind: "error", title: "maestro_compact_and_reset_sessions", text: formatError(error) });
    } finally {
      setCompacting(false);
    }
  }, [pushFeed]);

  const connected = useMemo(
    () => providers.filter((status) => status.state === "connected").length,
    [providers]
  );

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-rdc-bg text-slate-100">
      <AgentSidebar
        providers={providers}
        thinking={thinking}
        activeMission={activeMission}
        onLogin={handleLogin}
        onShow={handleShow}
        onHide={handleHide}
        onConfirmLogin={handleConfirmLogin}
        onCompactAndReset={handleCompactAndReset}
        compacting={compacting}
      />

      <main className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-3 border-b border-rdc-border bg-rdc-panel/60 px-6 py-3">
          <div className="min-w-0">
            <h1 className="truncate text-sm font-semibold tracking-wide">
              Blackboard Multi-Agente
            </h1>
            <p className="truncate text-[0.68rem] text-slate-500">{missionStatus}</p>
          </div>
          <div className="ml-auto flex items-center gap-3 text-[0.68rem] text-slate-400">
            <span className="rounded border border-rdc-border px-2 py-1">
              {connected}/{PROVIDER_IDS.length} sessões conectadas
            </span>
            <span className="rounded border border-rdc-border px-2 py-1">
              {consensus
                ? `consenso ${consensus.status} · ${(consensus.confidence * 100).toFixed(0)}%`
                : "sem consenso"}
            </span>
          </div>
        </header>

        <BlackboardFeed
          items={feed}
          planText={planText}
          onPlanChange={setPlanText}
          hasConsensus={consensus !== null}
        />

        <MissionComposer
          busy={busy}
          missionStatus={missionStatus}
          hasConsensus={consensus !== null}
          sendingPlan={sendingPlan}
          receipt={receipt}
          onSubmit={handleSubmit}
          onApprove={handleApprove}
          onRollback={handleRollback}
          rollingBack={rollingBack}
        />
      </main>
    </div>
  );
}
