"use client";
import React, { useState, useEffect } from "react";
import {
  Copy,
  Check,
  Terminal,
  ShieldCheck,
  Cpu,
  HelpCircle,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  AlertTriangle,
  RefreshCw,
  Sparkles,
  Monitor,
  CheckCircle2,
  FolderCheck,
  Activity,
  ArrowRight,
  AlertCircle
} from "lucide-react";

export const OFFICIAL_INSTALL_CMD =
  "$p=\"$env:TEMP\\remotecontroller-install.ps1\"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p";
export const OFFICIAL_MCP_URL = "https://andredozero.webajato.com.br/mcp";
export const EXECUTION_POLICY_CMD =
  "powershell -ExecutionPolicy Bypass -Command \"$p=\\\"$env:TEMP\\remotecontroller-install.ps1\\\"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p\"";
export const CHATGPT_URL = "https://chatgpt.com";
export const TEST_PROMPT_EXAMPLE =
  "Olá! Verifique se meu computador está online e crie um arquivo boas_vindas.txt na minha área de trabalho.";
export const RESTART_AGENT_CMD =
  "& \"$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\device-agent.exe\" run";
export const STATUS_AGENT_CMD =
  "& \"$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\device-agent.exe\" status";
export const MONITOR_CMD =
  "%LOCALAPPDATA%\\Programs\\RemoteDesktopController\\rdc-monitor.exe";

export interface DeviceInfo {
  device_id: string;
  name: string;
  online: boolean;
  revoked?: boolean;
  last_seen?: number;
}

export function WindowsInstallerCard(props?: { initialDevice?: DeviceInfo }) {
  const [copiedMain, setCopiedMain] = useState(false);
  const [copiedBypass, setCopiedBypass] = useState(false);
  const [copiedPrompt, setCopiedPrompt] = useState(false);
  const [copiedRestart, setCopiedRestart] = useState(false);
  const [copiedStatus, setCopiedStatus] = useState(false);
  const [copiedMonitor, setCopiedMonitor] = useState(false);
  const [copiedMcpUrl, setCopiedMcpUrl] = useState(false);
  const [showTutorial, setShowTutorial] = useState(false);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [openAccordions, setOpenAccordions] = useState<Record<string, boolean>>({
    policy: true,
    "code-used": true,
    offline: true,
    smartscreen: true,
    timeout: true
  });
  const [onlineDevice, setOnlineDevice] = useState<DeviceInfo | null>(
    props?.initialDevice?.online ? props.initialDevice : null
  );
  const [isCheckingStatus, setIsCheckingStatus] = useState(false);

  // Polling dinâmico para detectar dispositivo online
  useEffect(() => {
    let mounted = true;
    async function checkDevices() {
      if (typeof window === "undefined") return;
      try {
        setIsCheckingStatus(true);
        const res = await fetch("/api/devices", { cache: "no-store" });
        if (res.ok) {
          const data = (await res.json()) as { devices?: DeviceInfo[] };
          if (mounted && data.devices && data.devices.length > 0) {
            const active = data.devices.find((d) => d.online && !d.revoked);
            if (active) {
              setOnlineDevice(active);
            }
          }
        }
      } catch {
        // Silencioso em caso de falha de rede
      } finally {
        if (mounted) setIsCheckingStatus(false);
      }
    }

    if (!onlineDevice) {
      void checkDevices();
      const interval = setInterval(checkDevices, 10000);
      return () => {
        mounted = false;
        clearInterval(interval);
      };
    }
  }, [onlineDevice]);

  async function copyToClipboard(text: string, setter: (val: boolean) => void) {
    try {
      if (typeof navigator !== "undefined" && navigator.clipboard) {
        await navigator.clipboard.writeText(text);
        setter(true);
        setTimeout(() => setter(false), 3000);
      }
    } catch {}
  }

  const toggleAccordion = (key: string) => {
    setOpenAccordions((prev) => ({
      ...prev,
      [key]: !prev[key]
    }));
  };

  return (
    <section
      className="surface-card installer-card"
      style={{
        marginTop: "1.5rem",
        border: "1px solid #1e293b",
        background: "#0b0f19",
        borderRadius: "14px",
        padding: "1.75rem",
        color: "#f8fafc",
        boxShadow: "0 10px 30px rgba(0, 0, 0, 0.25)"
      }}
    >
      {/* BANNER DINÂMICO DE STATUS ONLINE */}
      {onlineDevice && (
        <div
          data-testid="online-banner"
          style={{
            background: "linear-gradient(90deg, rgba(16, 185, 129, 0.15), rgba(5, 150, 105, 0.25))",
            border: "1px solid #10b981",
            borderRadius: "10px",
            padding: "1rem 1.25rem",
            marginBottom: "1.5rem",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            flexWrap: "wrap",
            gap: "1rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.85rem" }}>
            <span
              style={{
                width: "12px",
                height: "12px",
                borderRadius: "50%",
                background: "#10b981",
                boxShadow: "0 0 10px #10b981",
                display: "inline-block",
                animation: "pulse 2s infinite"
              }}
            />
            <div>
              <strong style={{ color: "#ecfdf5", fontSize: "1.05rem" }}>
                ✅ Seu computador {onlineDevice.name || "conectado"} já está online e pronto para uso!
              </strong>
              <p style={{ margin: "0.2rem 0 0 0", color: "#a7f3d0", fontSize: "0.85rem" }}>
                O túnel seguro com o Remote Controller está ativo. A IA já pode ler seus projetos e executar tarefas locais.
              </p>
            </div>
          </div>
          <a
            href={CHATGPT_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="button button-small"
            style={{
              background: "#10b981",
              color: "#022c22",
              fontWeight: 700,
              display: "inline-flex",
              alignItems: "center",
              gap: "0.4rem",
              textDecoration: "none",
              borderRadius: "6px",
              padding: "0.45rem 0.9rem",
              fontSize: "0.85rem"
            }}
          >
            Abrir ChatGPT agora <ExternalLink size={14} />
          </a>
        </div>
      )}

      {/* CABEÇALHO DO ONBOARDING */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", flexWrap: "wrap", gap: "1rem" }}>
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "0.35rem" }}>
            <span
              className="pill"
              style={{
                background: "rgba(16, 185, 129, 0.15)",
                color: "#10b981",
                borderColor: "rgba(16, 185, 129, 0.3)",
                fontSize: "0.75rem",
                fontWeight: 700,
                letterSpacing: "0.08em"
              }}
            >
              GUIA VISUAL DE PRIMEIRO ACESSO
            </span>
            <span
              className="pill"
              style={{
                background: "rgba(56, 189, 248, 0.12)",
                color: "#38bdf8",
                borderColor: "rgba(56, 189, 248, 0.3)",
                fontSize: "0.75rem",
                fontWeight: 700
              }}
            >
              <ShieldCheck size={13} style={{ display: "inline", verticalAlign: "middle", marginRight: "3px" }} />
              Verificado SHA-256
            </span>
          </div>
          <h2 style={{ fontSize: "1.65rem", margin: "0.25rem 0 0.5rem 0", color: "#f8fafc", fontWeight: 700 }}>
            Instalar no Windows em 1 Passo
          </h2>
          <p style={{ color: "#94a3b8", margin: 0, fontSize: "0.95rem", maxWidth: "680px", lineHeight: "1.5" }}>
            Siga a jornada simples de 4 passos abaixo para conectar seu computador à IA em menos de 2 minutos.
            Sem compilações manuais e sem permissões de administrador.
          </p>
        </div>

        <button
          type="button"
          onClick={() => setShowDiagnostics(!showDiagnostics)}
          className="button button-outline button-small"
          style={{
            borderColor: showDiagnostics ? "#f59e0b" : "#334155",
            color: showDiagnostics ? "#fbbf24" : "#94a3b8",
            display: "inline-flex",
            alignItems: "center",
            gap: "0.45rem",
            background: showDiagnostics ? "rgba(245, 158, 11, 0.08)" : "transparent",
            fontSize: "0.85rem"
          }}
        >
          <AlertCircle size={15} />
          {showDiagnostics ? "Ocultar Diagnóstico" : "Central de Diagnóstico & Dúvidas"}
        </button>
      </div>

      {/* JORNADA DO PRIMEIRO ACESSO (4 CARDS MODERNOS) */}
      <div style={{ display: "flex", flexDirection: "column", gap: "1.25rem", marginTop: "1.75rem" }}>
        
        {/* 🔹 PASSO 1: COPIAR O COMANDO DE PAREAMENTO SEGURO */}
        <div
          data-step="1"
          style={{
            background: "rgba(15, 23, 42, 0.8)",
            border: "1px solid #1e293b",
            borderRadius: "12px",
            padding: "1.35rem",
            position: "relative"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: "0.5rem", marginBottom: "0.75rem" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
              <span
                style={{
                  background: "#10b981",
                  color: "#022c22",
                  width: "26px",
                  height: "26px",
                  borderRadius: "50%",
                  display: "inline-grid",
                  placeItems: "center",
                  fontWeight: 800,
                  fontSize: "0.85rem"
                }}
              >
                1
              </span>
              <strong style={{ fontSize: "1.1rem", color: "#f8fafc" }}>
                1. Cole no PowerShell (Copiar Comando Seguro)
              </strong>
            </div>

            <div style={{ display: "flex", gap: "0.4rem", flexWrap: "wrap" }}>
              <span style={{ fontSize: "0.75rem", background: "rgba(16, 185, 129, 0.1)", color: "#10b981", padding: "0.2rem 0.5rem", borderRadius: "4px", border: "1px solid rgba(16, 185, 129, 0.2)" }}>
                Isolado em %LOCALAPPDATA%
              </span>
              <span style={{ fontSize: "0.75rem", background: "rgba(56, 189, 248, 0.1)", color: "#38bdf8", padding: "0.2rem 0.5rem", borderRadius: "4px", border: "1px solid rgba(56, 189, 248, 0.2)" }}>
                Sem Admin
              </span>
            </div>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.9rem", margin: "0 0 0.85rem 0", lineHeight: "1.5" }}>
            Esse comando baixa e inicia o conector leve no seu computador, criando um túnel seguro e criptografado exclusivo da sua conta.
            O instalador <strong>não mexe no sistema global</strong> e instala isolado na pasta de usuário (<code>%LOCALAPPDATA%</code>),
            dispensando privilégios de Administrador.
          </p>

          <div
            style={{
              background: "#030712",
              border: "1px solid #1e293b",
              borderRadius: "8px",
              padding: "0.85rem 1rem",
              display: "flex",
              alignItems: "center",
              gap: "0.75rem",
              overflowX: "auto"
            }}
          >
            <Terminal size={18} style={{ color: "#38bdf8", flexShrink: 0 }} />
            <code style={{ flex: 1, fontFamily: "Consolas, monospace", color: "#38bdf8", fontSize: "0.88rem", whiteSpace: "nowrap" }}>
              {OFFICIAL_INSTALL_CMD}
            </code>
            <button
              type="button"
              onClick={() => copyToClipboard(OFFICIAL_INSTALL_CMD, setCopiedMain)}
              style={{
                flexShrink: 0,
                background: copiedMain ? "#10b981" : "#1e293b",
                color: "#fff",
                border: "none",
                padding: "0.5rem 1rem",
                borderRadius: "6px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.4rem",
                fontWeight: 600,
                fontSize: "0.85rem",
                transition: "all 0.2s"
              }}
            >
              {copiedMain ? <Check size={14} /> : <Copy size={14} />}
              {copiedMain ? "Copiado!" : "Copiar comando"}
            </button>
          </div>
        </div>

        {/* 🔹 PASSO 2: EXECUTAR NO WINDOWS (SEM COMPLICAÇÃO) */}
        <div
          data-step="2"
          style={{
            background: "rgba(15, 23, 42, 0.8)",
            border: "1px solid #1e293b",
            borderRadius: "12px",
            padding: "1.35rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.6rem", marginBottom: "0.75rem" }}>
            <span
              style={{
                background: "#38bdf8",
                color: "#082f49",
                width: "26px",
                height: "26px",
                borderRadius: "50%",
                display: "inline-grid",
                placeItems: "center",
                fontWeight: 800,
                fontSize: "0.85rem"
              }}
            >
              2
            </span>
            <strong style={{ fontSize: "1.1rem", color: "#f8fafc" }}>
              2. Pareie com 1 Clique (Executar no Windows)
            </strong>
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))", gap: "1.25rem", alignItems: "start" }}>
            {/* Instruções passo a passo */}
            <div>
              <ol style={{ paddingLeft: "1.2rem", margin: 0, color: "#cbd5e1", fontSize: "0.9rem", lineHeight: "1.7" }}>
                <li>
                  Pressione a tecla <strong>Windows</strong> no teclado e digite <strong>PowerShell</strong>.
                </li>
                <li>
                  Clique com o botão esquerdo para abrir (<strong>NÃO precisa ser &quot;Executar como Administrador&quot;</strong>).
                </li>
                <li>
                  Cole o comando com <kbd style={{ background: "#1e293b", padding: "0.15rem 0.4rem", borderRadius: "4px", fontSize: "0.8rem", color: "#93c5fd" }}>Ctrl + V</kbd> (ou clique com o botão direito na janela azul/preta) e aperte <strong>Enter</strong>.
                </li>
              </ol>

              {/* Mockup do Menu Iniciar */}
              <div
                style={{
                  marginTop: "0.9rem",
                  background: "#1e293b",
                  borderRadius: "8px",
                  padding: "0.75rem",
                  border: "1px solid #334155",
                  display: "flex",
                  alignItems: "center",
                  gap: "0.75rem"
                }}
              >
                <div style={{ width: "32px", height: "32px", background: "#0284c7", borderRadius: "6px", display: "grid", placeItems: "center", color: "#fff" }}>
                  <Terminal size={18} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: "0.85rem", color: "#f1f5f9", fontWeight: 600 }}>Windows PowerShell</div>
                  <div style={{ fontSize: "0.75rem", color: "#94a3b8" }}>Aplicativo · Pressione Enter para abrir</div>
                </div>
                <span style={{ fontSize: "0.75rem", color: "#10b981", background: "rgba(16, 185, 129, 0.15)", padding: "0.2rem 0.5rem", borderRadius: "4px" }}>
                  Sem Admin
                </span>
              </div>
            </div>

            {/* Preview do Terminal de Sucesso */}
            <div
              style={{
                background: "#020617",
                border: "1px solid #334155",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <div style={{ background: "#0f172a", padding: "0.4rem 0.75rem", display: "flex", alignItems: "center", justifyContent: "space-between", borderBottom: "1px solid #1e293b" }}>
                <span style={{ fontSize: "0.75rem", color: "#94a3b8", fontFamily: "Consolas, monospace" }}>Windows PowerShell</span>
                <div style={{ display: "flex", gap: "5px" }}>
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#475569" }} />
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#475569" }} />
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#ef4444" }} />
                </div>
              </div>
              <div style={{ padding: "0.85rem", fontFamily: "Consolas, monospace", fontSize: "0.8rem", lineHeight: "1.6" }}>
                <div style={{ color: "#64748b" }}>PS C:\Users\Notebook&gt; [comando executado]</div>
                <div style={{ color: "#38bdf8" }}>==&gt; Baixando pacote verificado e manifesto oficial...</div>
                <div style={{ color: "#10b981", fontWeight: 700 }}> [OK] Instalado com sucesso!</div>
                <div style={{ color: "#10b981", fontWeight: 700 }}> [OK] Computador Pareado com Sucesso!</div>
                <div style={{ color: "#a855f7" }}>==&gt; Agente ativo em segundo plano.</div>
              </div>
            </div>
          </div>
        </div>

        {/* 🔹 PASSO 3: CONECTAR E AUTORIZAR NO CHATGPT */}
        <div
          data-step="3"
          style={{
            background: "rgba(15, 23, 42, 0.8)",
            border: "1px solid #1e293b",
            borderRadius: "12px",
            padding: "1.35rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: "0.5rem", marginBottom: "0.75rem" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
              <span
                style={{
                  background: "#a855f7",
                  color: "#fff",
                  width: "26px",
                  height: "26px",
                  borderRadius: "50%",
                  display: "inline-grid",
                  placeItems: "center",
                  fontWeight: 800,
                  fontSize: "0.85rem"
                }}
              >
                3
              </span>
              <strong style={{ fontSize: "1.1rem", color: "#f8fafc" }}>
                3. Conecte ao ChatGPT (Abrir Meu Agente)
              </strong>
            </div>

            <a
              href={CHATGPT_URL}
              target="_blank"
              rel="noopener noreferrer"
              style={{
                background: "#10b981",
                color: "#022c22",
                fontWeight: 700,
                padding: "0.45rem 1rem",
                borderRadius: "6px",
                textDecoration: "none",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.4rem",
                fontSize: "0.85rem",
                boxShadow: "0 0 15px rgba(16, 185, 129, 0.25)"
              }}
            >
              Abrir o ChatGPT / Meu Agente <ExternalLink size={14} />
            </a>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.9rem", margin: "0 0 0.85rem 0", lineHeight: "1.5" }}>
            Adicione o endpoint MCP no ChatGPT e use a IA para ler e analisar seus projetos locais com segurança.
          </p>

          {/* BOX DE ALERTA DOURADO: ATENÇÃO AO BOTÃO PERMITIR */}
          <div
            style={{
              background: "linear-gradient(90deg, rgba(245, 158, 11, 0.12), rgba(217, 119, 6, 0.18))",
              border: "1px solid #f59e0b",
              borderRadius: "8px",
              padding: "1rem 1.2rem",
              marginTop: "0.75rem"
            }}
          >
            <div style={{ display: "flex", alignItems: "flex-start", gap: "0.75rem" }}>
              <AlertTriangle size={20} style={{ color: "#fbbf24", flexShrink: 0, marginTop: "2px" }} />
              <div>
                <strong style={{ color: "#fef3c7", fontSize: "0.95rem", display: "block", marginBottom: "0.35rem" }}>
                  PONTO CRÍTICO: Clique em &quot;Permitir&quot; (ou &quot;Always Allow&quot;) no ChatGPT
                </strong>
                <p style={{ margin: 0, color: "#fde68a", fontSize: "0.88rem", lineHeight: "1.55" }}>
                  Na primeira vez que você pedir algo, o ChatGPT exibirá uma janela de segurança perguntando se você autoriza a conexão.
                  Clique em <strong>Permitir</strong> (ou <strong>Sempre permitir</strong>). Se você fechar ou ignorar essa janela, a IA não conseguirá falar com o seu PC!
                </p>
              </div>
            </div>
          </div>
        </div>

        {/* 🔹 PASSO 4: O TESTE DE BOAS-VINDAS & RDC-MONITOR */}
        <div
          data-step="4"
          style={{
            background: "rgba(15, 23, 42, 0.8)",
            border: "1px solid #1e293b",
            borderRadius: "12px",
            padding: "1.35rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.6rem", marginBottom: "0.75rem" }}>
            <span
              style={{
                background: "#f59e0b",
                color: "#451a03",
                width: "26px",
                height: "26px",
                borderRadius: "50%",
                display: "inline-grid",
                placeItems: "center",
                fontWeight: 800,
                fontSize: "0.85rem"
              }}
            >
              4
            </span>
            <strong style={{ fontSize: "1.1rem", color: "#f8fafc" }}>
              4. O Teste de Boas-Vindas & Monitor em Tempo Real
            </strong>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.9rem", margin: "0 0 0.85rem 0", lineHeight: "1.5" }}>
            Envie esta mensagem no ChatGPT para confirmar que o conector local está respondendo instantaneamente:
          </p>

          {/* Balão de sugestão de prompt */}
          <div
            style={{
              background: "#030712",
              border: "1px solid #22c55e",
              borderRadius: "8px",
              padding: "0.9rem 1.1rem",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "0.75rem",
              flexWrap: "wrap"
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.6rem", flex: 1, minWidth: "240px" }}>
              <Sparkles size={18} style={{ color: "#22c55e", flexShrink: 0 }} />
              <span style={{ color: "#f0fdf4", fontStyle: "italic", fontSize: "0.9rem" }}>
                &ldquo;{TEST_PROMPT_EXAMPLE}&rdquo;
              </span>
            </div>
            <button
              type="button"
              onClick={() => copyToClipboard(TEST_PROMPT_EXAMPLE, setCopiedPrompt)}
              style={{
                background: copiedPrompt ? "#22c55e" : "#1e293b",
                color: "#fff",
                border: "none",
                padding: "0.4rem 0.85rem",
                borderRadius: "6px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.4rem",
                fontWeight: 600,
                fontSize: "0.82rem"
              }}
            >
              {copiedPrompt ? <Check size={14} /> : <Copy size={14} />}
              {copiedPrompt ? "Copiado!" : "Copiar Exemplo"}
            </button>
          </div>

          {/* Dica Extra: rdc-monitor.exe */}
          <div
            style={{
              marginTop: "1rem",
              padding: "0.85rem 1rem",
              background: "rgba(30, 41, 59, 0.5)",
              border: "1px solid #334155",
              borderRadius: "8px",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "0.75rem",
              flexWrap: "wrap"
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
              <Activity size={17} style={{ color: "#38bdf8", flexShrink: 0 }} />
              <div style={{ fontSize: "0.85rem", color: "#cbd5e1" }}>
                <strong>Dica Extra (Monitor Opcional):</strong> Deseja assistir cada ação e diff de linha na tela em tempo real? Abra o <code>rdc-monitor.exe</code>.
              </div>
            </div>
            <button
              type="button"
              onClick={() => copyToClipboard(MONITOR_CMD, setCopiedMonitor)}
              style={{
                background: copiedMonitor ? "#10b981" : "#1e293b",
                color: "#fff",
                border: "none",
                padding: "0.35rem 0.75rem",
                borderRadius: "6px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.35rem",
                fontSize: "0.8rem",
                fontWeight: 600
              }}
            >
              {copiedMonitor ? <Check size={13} /> : <Copy size={13} />}
              {copiedMonitor ? "Copiado!" : "Copiar comando rdc-monitor"}
            </button>
          </div>
        </div>

      </div>

      {/* TUTORIAL MCP EXPANSÍVEL (BACKWARDS COMPATIBILITY) */}
      <div style={{ marginTop: "1.5rem", borderTop: "1px solid rgba(255,255,255,0.08)", paddingTop: "1rem" }}>
        <button
          type="button"
          onClick={() => setShowTutorial(!showTutorial)}
          style={{
            background: "none",
            border: "none",
            color: "#38bdf8",
            cursor: "pointer",
            display: "inline-flex",
            alignItems: "center",
            gap: "0.5rem",
            fontWeight: 600,
            fontSize: "0.92rem",
            padding: 0
          }}
        >
          <HelpCircle size={18} />
          {showTutorial ? "Ocultar tutorial de conexão MCP ao ChatGPT" : "Como conectar o Remote Controller ao ChatGPT (MCP)"}
          {showTutorial ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
        </button>

        {showTutorial && <McpChatGPTTutorial />}
      </div>

      {/* CENTRAL DE DIAGNÓSTICO & ERROS COMUNS (ACORDEÃO / TROUBLESHOOTING) */}
      {(showDiagnostics || true) && (
        <section
          style={{
            marginTop: "1.75rem",
            borderTop: "1px solid #1e293b",
            paddingTop: "1.5rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "1rem" }}>
            <AlertCircle size={20} style={{ color: "#f59e0b" }} />
            <h3 style={{ fontSize: "1.2rem", margin: 0, color: "#f1f5f9", fontWeight: 700 }}>
              Central de Diagnóstico &amp; Solução de Erros Comuns
            </h3>
          </div>
          <p style={{ color: "#94a3b8", fontSize: "0.88rem", marginTop: 0, marginBottom: "1.25rem" }}>
            Encontrou algum imprevisto? Clique no item abaixo para ver a solução imediata em 1 clique:
          </p>

          <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
            
            {/* 1. EXECUTION POLICY */}
            <div
              style={{
                background: "#0f172a",
                border: "1px solid #1e293b",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <button
                type="button"
                onClick={() => toggleAccordion("policy")}
                style={{
                  width: "100%",
                  padding: "0.85rem 1rem",
                  background: "transparent",
                  border: "none",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  color: "#f8fafc",
                  fontWeight: 600,
                  fontSize: "0.9rem",
                  cursor: "pointer",
                  textAlign: "left"
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                  <span style={{ color: "#ef4444" }}>🔴</span>
                  Erro: &quot;A execução de scripts foi desabilitada neste sistema (ExecutionPolicy)&quot;
                </span>
                {openAccordions["policy"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
              </button>

              {openAccordions["policy"] && (
                <div style={{ padding: "0.85rem 1rem 1rem 1rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                  <p style={{ margin: "0 0 0.5rem 0", fontSize: "0.85rem", color: "#94a3b8" }}>
                    <strong>Causa:</strong> O Windows bloqueou scripts PowerShell por política de segurança padrão.
                  </p>
                  <p style={{ margin: "0 0 0.75rem 0", fontSize: "0.85rem", color: "#cbd5e1" }}>
                    <strong>Solução:</strong> Use o comando oficial alternativo que inclui <code>-ExecutionPolicy Bypass</code> sem alterar regras globais do sistema:
                  </p>
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.6rem 0.85rem", borderRadius: "6px", gap: "0.5rem", border: "1px solid #1e293b" }}>
                    <code style={{ flex: 1, color: "#38bdf8", fontSize: "0.82rem", whiteSpace: "nowrap", overflowX: "auto" }}>
                      {EXECUTION_POLICY_CMD}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard(EXECUTION_POLICY_CMD, setCopiedBypass)}
                      style={{
                        background: copiedBypass ? "#10b981" : "#1e293b",
                        color: "#fff",
                        border: "none",
                        padding: "0.35rem 0.75rem",
                        borderRadius: "4px",
                        fontSize: "0.8rem",
                        cursor: "pointer",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.3rem"
                      }}
                    >
                      {copiedBypass ? <Check size={13} /> : <Copy size={13} />}
                      {copiedBypass ? "Copiado!" : "Copiar"}
                    </button>
                  </div>
                </div>
              )}
            </div>

            {/* 2. CÓDIGO JÁ UTILIZADO */}
            <div
              style={{
                background: "#0f172a",
                border: "1px solid #1e293b",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <button
                type="button"
                onClick={() => toggleAccordion("code-used")}
                style={{
                  width: "100%",
                  padding: "0.85rem 1rem",
                  background: "transparent",
                  border: "none",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  color: "#f8fafc",
                  fontWeight: 600,
                  fontSize: "0.9rem",
                  cursor: "pointer",
                  textAlign: "left"
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                  <span style={{ color: "#ef4444" }}>🔴</span>
                  Erro: &quot;Código de pareamento já utilizado (Already used) ou expirado&quot;
                </span>
                {openAccordions["code-used"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
              </button>

              {openAccordions["code-used"] && (
                <div style={{ padding: "0.85rem 1rem 1rem 1rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                  <p style={{ margin: "0 0 0.5rem 0", fontSize: "0.85rem", color: "#94a3b8" }}>
                    <strong>Causa:</strong> O código de pareamento dura 10 minutos ou já foi consumido por outra máquina.
                  </p>
                  <p style={{ margin: "0 0 0.75rem 0", fontSize: "0.85rem", color: "#cbd5e1" }}>
                    <strong>Solução:</strong> Reabra a página de pareamento ou reexecute o comando no PowerShell para gerar um novo código de uso único.
                  </p>
                  <a
                    href="/pair"
                    style={{
                      background: "#1e293b",
                      color: "#38bdf8",
                      border: "1px solid #334155",
                      padding: "0.45rem 0.9rem",
                      borderRadius: "6px",
                      display: "inline-flex",
                      alignItems: "center",
                      gap: "0.4rem",
                      fontSize: "0.85rem",
                      textDecoration: "none",
                      fontWeight: 600
                    }}
                  >
                    <RefreshCw size={14} /> Ir para Página de Pareamento
                  </a>
                </div>
              )}
            </div>

            {/* 3. DISPOSITIVO OFFLINE */}
            <div
              style={{
                background: "#0f172a",
                border: "1px solid #1e293b",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <button
                type="button"
                onClick={() => toggleAccordion("offline")}
                style={{
                  width: "100%",
                  padding: "0.85rem 1rem",
                  background: "transparent",
                  border: "none",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  color: "#f8fafc",
                  fontWeight: 600,
                  fontSize: "0.9rem",
                  cursor: "pointer",
                  textAlign: "left"
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                  <span style={{ color: "#ef4444" }}>🔴</span>
                  Erro: &quot;O ChatGPT diz que o dispositivo está offline&quot;
                </span>
                {openAccordions["offline"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
              </button>

              {openAccordions["offline"] && (
                <div style={{ padding: "0.85rem 1rem 1rem 1rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                  <p style={{ margin: "0 0 0.5rem 0", fontSize: "0.85rem", color: "#94a3b8" }}>
                    <strong>Causa:</strong> A janela do conector foi fechada, o Windows suspendeu a placa de rede ou o computador reiniciou.
                  </p>
                  <p style={{ margin: "0 0 0.75rem 0", fontSize: "0.85rem", color: "#cbd5e1" }}>
                    <strong>Solução:</strong> Reabra o agente em segundo plano ou verifique seu status com um comando no PowerShell:
                  </p>
                  <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem" }}>
                    <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.5rem 0.8rem", borderRadius: "6px", gap: "0.5rem" }}>
                      <code style={{ flex: 1, color: "#38bdf8", fontSize: "0.82rem" }}>{RESTART_AGENT_CMD}</code>
                      <button
                        type="button"
                        onClick={() => copyToClipboard(RESTART_AGENT_CMD, setCopiedRestart)}
                        style={{ background: copiedRestart ? "#10b981" : "#1e293b", color: "#fff", border: "none", padding: "0.3rem 0.65rem", borderRadius: "4px", fontSize: "0.75rem", cursor: "pointer" }}
                      >
                        {copiedRestart ? "Copiado!" : "Copiar"}
                      </button>
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* 4. WINDOWS DEFENDER / SMARTSCREEN */}
            <div
              style={{
                background: "#0f172a",
                border: "1px solid #1e293b",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <button
                type="button"
                onClick={() => toggleAccordion("smartscreen")}
                style={{
                  width: "100%",
                  padding: "0.85rem 1rem",
                  background: "transparent",
                  border: "none",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  color: "#f8fafc",
                  fontWeight: 600,
                  fontSize: "0.9rem",
                  cursor: "pointer",
                  textAlign: "left"
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                  <span style={{ color: "#ef4444" }}>🔴</span>
                  Erro: &quot;Aviso do Windows Defender / SmartScreen&quot;
                </span>
                {openAccordions["smartscreen"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
              </button>

              {openAccordions["smartscreen"] && (
                <div style={{ padding: "0.85rem 1rem 1rem 1rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                  <p style={{ margin: "0 0 0.5rem 0", fontSize: "0.85rem", color: "#94a3b8" }}>
                    <strong>Causa:</strong> Esta é uma nova versão do executável e a Microsoft ainda não acumulou telemetria suficiente para aprová-la silenciosamente.
                  </p>
                  <p style={{ margin: "0 0 0.75rem 0", fontSize: "0.85rem", color: "#cbd5e1" }}>
                    <strong>Solução:</strong>
                  </p>
                  <ol style={{ paddingLeft: "1.2rem", margin: 0, color: "#cbd5e1", fontSize: "0.85rem", lineHeight: "1.6" }}>
                    <li>Na janela azul do Windows SmartScreen, clique no link <strong>&quot;Mais informações&quot;</strong>.</li>
                    <li>Em seguida, clique no botão <strong>&quot;Executar assim mesmo&quot;</strong>.</li>
                    <li>O instalador é 100% verificado com hash criptográfico SHA-256 e roda isolado na sua pasta local.</li>
                  </ol>
                </div>
              )}
            </div>

            {/* 5. CARREGAMENTO INFINITO / TIMEOUT NO CHATGPT */}
            <div
              style={{
                background: "#0f172a",
                border: "1px solid #1e293b",
                borderRadius: "8px",
                overflow: "hidden"
              }}
            >
              <button
                type="button"
                onClick={() => toggleAccordion("timeout")}
                style={{
                  width: "100%",
                  padding: "0.85rem 1rem",
                  background: "transparent",
                  border: "none",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  color: "#f8fafc",
                  fontWeight: 600,
                  fontSize: "0.9rem",
                  cursor: "pointer",
                  textAlign: "left"
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                  <span style={{ color: "#ef4444" }}>🔴</span>
                  Erro: &quot;O ChatGPT fica carregando infinitamente e dá erro de timeout&quot;
                </span>
                {openAccordions["timeout"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
              </button>

              {openAccordions["timeout"] && (
                <div style={{ padding: "0.85rem 1rem 1rem 1rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                  <p style={{ margin: "0 0 0.5rem 0", fontSize: "0.85rem", color: "#94a3b8" }}>
                    <strong>Causa:</strong> Você não clicou no botão &quot;Permitir&quot; da janela de autorização do ChatGPT ou a janela ficou oculta em segundo plano.
                  </p>
                  <p style={{ margin: "0 0 0.75rem 0", fontSize: "0.85rem", color: "#cbd5e1" }}>
                    <strong>Solução:</strong> Atualize a página do ChatGPT (<kbd style={{ background: "#1e293b", padding: "0.1rem 0.3rem", borderRadius: "3px" }}>F5</kbd>) e repita a pergunta. Assim que o balão aparecer logo acima da caixa de resposta, clique em <strong>&quot;Permitir&quot;</strong> ou <strong>&quot;Sempre permitir&quot;</strong>.
                  </p>
                </div>
              )}
            </div>

          </div>
        </section>
      )}
    </section>
  );
}

export const OnboardingDiagnosticGuide = WindowsInstallerCard;

export function McpChatGPTTutorial() {
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function copyMcpUrl() {
    try {
      if (typeof navigator !== "undefined" && navigator.clipboard) {
        await navigator.clipboard.writeText(OFFICIAL_MCP_URL);
        setCopiedUrl(true);
        setTimeout(() => setCopiedUrl(false), 3000);
      }
    } catch {}
  }

  return (
    <div style={{ marginTop: "1rem", padding: "1.25rem", background: "rgba(15, 23, 42, 0.7)", borderRadius: "8px", border: "1px solid #334155" }}>
      <h3 style={{ fontSize: "1.1rem", margin: "0 0 0.75rem 0", color: "#f1f5f9", display: "flex", alignItems: "center", gap: "0.5rem" }}>
        <Cpu size={18} style={{ color: "#10b981" }} /> Tutorial: Conectar como Servidor MCP no ChatGPT
      </h3>
      <ol style={{ paddingLeft: "1.25rem", margin: 0, color: "#cbd5e1", fontSize: "0.9rem", lineHeight: "1.6" }}>
        <li>
          Abra o <strong>ChatGPT</strong> (Desktop ou Web) e acesse <strong>Configurações</strong> (Settings).
        </li>
        <li>
          Vá para a seção <strong>Connected Apps (Aplicativos Conectados)</strong> ou <strong>MCP Servers</strong> e clique em <strong>Add New Server (Adicionar Servidor)</strong>.
        </li>
        <li>
          Insira a <strong>URL oficial do servidor MCP</strong>:
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", margin: "0.5rem 0", background: "#0b0f19", padding: "0.5rem 0.75rem", borderRadius: "6px", width: "fit-content" }}>
            <code style={{ color: "#38bdf8", fontWeight: 600 }}>{OFFICIAL_MCP_URL}</code>
            <button
              type="button"
              onClick={copyMcpUrl}
              style={{ background: "none", border: "none", color: copiedUrl ? "#10b981" : "#94a3b8", cursor: "pointer", display: "flex", alignItems: "center", gap: "0.25rem", fontSize: "0.8rem" }}
            >
              {copiedUrl ? <Check size={13} /> : <Copy size={13} />}
              {copiedUrl ? "Copiado" : "Copiar"}
            </button>
          </div>
        </li>
        <li>
          Selecione <strong>OAuth</strong> (ou autorize pelo navegador quando solicitado). O ChatGPT vinculará sua conta de forma segura.
        </li>
        <li>
          <strong>Pronto!</strong> Suas conversas no ChatGPT poderão ler arquivos do computador autorizado, consultar estrutura de código e executar tarefas remotas com proteção total.
        </li>
      </ol>
      <div style={{ marginTop: "1rem", fontSize: "0.85rem", color: "#94a3b8", display: "flex", alignItems: "center", gap: "0.5rem" }}>
        <ShieldCheck size={16} style={{ color: "#10b981" }} />
        <span>Por padrão, o computador opera em modo <strong>somente leitura</strong>. Nenhuma escrita ou execução ocorre sem sua permissão explícita.</span>
      </div>
    </div>
  );
}
