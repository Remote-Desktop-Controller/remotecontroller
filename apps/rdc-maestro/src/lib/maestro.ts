// Camada de integração tipada com o backend Rust do Maestro (Fases 1, 3 e 4).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

export const PROVIDER_IDS = [
  "chatgpt",
  "claude",
  "gemini",
  "deepseek",
  "qwen",
] as const;
export type ProviderId = (typeof PROVIDER_IDS)[number];

export const PROVIDER_LABELS: Record<string, string> = {
  chatgpt: "ChatGPT",
  claude: "Claude",
  gemini: "Gemini",
  deepseek: "DeepSeek",
  qwen: "Qwen",
};

export type ProviderState = "disconnected" | "authenticating" | "connected";

export interface ProviderStatus {
  id: string;
  name: string;
  url: string;
  state: ProviderState;
  visible: boolean;
  detail: string;
}

export interface ConsensusPoint {
  text: string;
  providers: string[];
  support: number;
}

export interface ProviderReply {
  provider: string;
  mission_id: string | null;
  text: string;
  ok: boolean;
  captured_at: number;
  latency_ms: number;
}

export interface DebateOutcome {
  mission_id: string;
  prompt: string;
  status: "consensus" | "partial" | "no_reply" | string;
  consensus: string;
  convergences: ConsensusPoint[];
  divergences: ConsensusPoint[];
  replies: ProviderReply[];
  missing: string[];
  quorum_met: boolean;
  coverage: number;
  confidence: number;
  elapsed_ms: number;
  started_at: number;
  finished_at: number;
}

export interface DebateProgress {
  mission_id: string;
  stage: string;
  provider: string | null;
  detail: string;
  received: number;
  expected: number;
  timestamp: number;
}

export interface PlanExecutionReceipt {
  endpoint: string;
  mission_id: string | null;
  encoding: string;
  bytes_written: number;
  dispatched_at: number;
  acknowledged: boolean;
  response: string | null;
}

export interface MaestroLog {
  level: "system" | "info" | "success" | "warn" | "error";
  message: string;
  timestamp: string;
}

/// Comandos IPC expostos pelas capabilities do app.
export const maestroApi = {
  startMission: (prompt: string) =>
    invoke<DebateOutcome>("maestro_start_mission", { prompt }),
  executePlan: (payload: string) =>
    invoke<PlanExecutionReceipt>("maestro_execute_plan", { payload }),
  lastConsensus: () => invoke<DebateOutcome | null>("maestro_get_last_consensus"),
  providerStatuses: () => invoke<ProviderStatus[]>("get_all_providers_status"),
  openLogin: (provider: ProviderId) =>
    invoke<ProviderStatus>("open_provider_login", { provider }),
  showProvider: (provider: ProviderId) =>
    invoke<ProviderStatus>("show_provider_window", { provider }),
  hideProvider: (provider: ProviderId) =>
    invoke<ProviderStatus>("hide_provider_window", { provider }),
  setProviderConnected: (provider: ProviderId) =>
    invoke<ProviderStatus>("set_provider_connected", { provider }),
};

/// Eventos emitidos pelo motor (orchestrator.rs) e pelas WebViews.
export const MAESTRO_EVENTS = {
  providerStatus: "provider-status-changed",
  progress: "maestro-debate-progress",
  partial: "maestro-reply-partial",
  consensus: "maestro-consensus-ready",
  failed: "maestro-debate-failed",
  dispatched: "maestro-plan-dispatched",
  log: "maestro-log",
} as const;

/// Assina um evento Tauri mantendo o handler sempre atualizado e desinscrevendo no unmount.
export function useTauriEvent<T>(event: string, handler: (payload: T) => void): void {
  const handlerRef = useRef(handler);

  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    listen<T>(event, (received) => handlerRef.current(received.payload))
      .then((fn) => {
        if (disposed) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((error: unknown) => {
        console.error(`[maestro] falha ao assinar o evento ${event}`, error);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [event]);
}

export function formatClock(epochMs: number): string {
  return new Date(epochMs).toLocaleTimeString("pt-BR", { hour12: false });
}

export function formatError(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return JSON.stringify(error);
}
