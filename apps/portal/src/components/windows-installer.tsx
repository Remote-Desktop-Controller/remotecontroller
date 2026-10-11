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
  AlertCircle,
  KeyRound,
  PlayCircle
} from "lucide-react";

export const OFFICIAL_INSTALL_CMD =
  "$p=\"$env:TEMP\\remotecontroller-install.ps1\"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p";
export const OFFICIAL_MCP_URL = "https://andredozero.webajato.com.br/mcp";
export const EXECUTION_POLICY_CMD =
  "powershell -ExecutionPolicy Bypass -Command \"$p=\\\"$env:TEMP\\remotecontroller-install.ps1\\\"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p\"";
export const ALTERNATIVE_POLICY_CMD =
  "Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force; $p=\"$env:TEMP\\remotecontroller-install.ps1\"; irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 -OutFile $p; & $p";
export const CHATGPT_URL = "https://chatgpt.com";
export const TEST_PROMPT_EXAMPLE =
  "Olá! Verifique se meu computador está online e crie um arquivo boas_vindas.txt na minha área de trabalho.";
export const RESTART_AGENT_CMD =
  "& \"$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\device-agent.exe\" run";
export const STATUS_AGENT_CMD =
  "& \"$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\device-agent.exe\" status";
export const MONITOR_CMD =
  "%LOCALAPPDATA%\\Programs\\RemoteDesktopController\\rdc-monitor.exe";
export const POWERSHELL_MONITOR_CMD =
  "& \"$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\rdc-monitor.exe\"";

export interface DeviceInfo {
  device_id: string;
  name: string;
  online: boolean;
  revoked?: boolean;
  last_seen?: number;
}

function generateRandomCode(): string {
  const chars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  let c1 = "";
  let c2 = "";
  for (let i = 0; i < 4; i++) {
    c1 += chars[Math.floor(Math.random() * chars.length)];
    c2 += chars[Math.floor(Math.random() * chars.length)];
  }
  return `${c1}-${c2}`;
}

export function WindowsInstallerCard(props?: {
  initialDevice?: DeviceInfo;
  onNewCodeGenerated?: (code: string) => void;
}) {
  const [copiedMain, setCopiedMain] = useState(false);
  const [copiedBypass, setCopiedBypass] = useState(false);
  const [copiedAltPolicy, setCopiedAltPolicy] = useState(false);
  const [copiedPrompt, setCopiedPrompt] = useState(false);
  const [copiedRestart, setCopiedRestart] = useState(false);
  const [copiedStatus, setCopiedStatus] = useState(false);
  const [copiedMonitor, setCopiedMonitor] = useState(false);
  const [copiedMonitorPs, setCopiedMonitorPs] = useState(false);
  const [copiedGeneratedCode, setCopiedGeneratedCode] = useState(false);
  const [copiedPairCmd, setCopiedPairCmd] = useState(false);

  const [generatedPairCode, setGeneratedPairCode] = useState<string | null>(null);
  const [generatedAt, setGeneratedAt] = useState<Date | null>(null);

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

  // Polling dinâmico contínuo para refletir conexão ou desconexão em tempo real
  useEffect(() => {
    let mounted = true;
    async function checkDevices() {
      if (typeof window === "undefined") return;
      try {
        setIsCheckingStatus(true);
        const res = await fetch("/api/devices", { cache: "no-store" });
        if (res.ok) {
          const data = (await res.json()) as { devices?: DeviceInfo[] };
          if (mounted) {
            if (data.devices && data.devices.length > 0) {
              const active = data.devices.find((d) => d.online && !d.revoked);
              setOnlineDevice(active || null);
            } else {
              setOnlineDevice(null);
            }
          }
        }
      } catch {
        // Silencioso em caso de falha transitória de rede
      } finally {
        if (mounted) setIsCheckingStatus(false);
      }
    }

    void checkDevices();
    const interval = setInterval(checkDevices, 8000);
    return () => {
      mounted = false;
      clearInterval(interval);
    };
  }, []);

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

  const handleGenerateNewCode = () => {
    const code = generateRandomCode();
    setGeneratedPairCode(code);
    setGeneratedAt(new Date());
    if (props?.onNewCodeGenerated) {
      props.onNewCodeGenerated(code);
    }
  };

  return (
    <section
      className="surface-card installer-card"
      style={{
        marginTop: "1.5rem",
        border: "1px solid #1e293b",
        background: "radial-gradient(ellipse at top right, #0f172a 0%, #090d16 100%)",
        borderRadius: "16px",
        padding: "1.75rem",
        color: "#f8fafc",
        boxShadow: "0 12px 36px rgba(0, 0, 0, 0.35)",
        position: "relative",
        overflow: "hidden"
      }}
    >
      {/* BANNER DINÂMICO DE STATUS ONLINE */}
      {onlineDevice && (
        <div
          data-testid="online-banner"
          style={{
            background: "linear-gradient(90deg, rgba(16, 185, 129, 0.18), rgba(5, 150, 105, 0.28))",
            border: "1px solid #10b981",
            borderRadius: "12px",
            padding: "1.1rem 1.35rem",
            marginBottom: "1.75rem",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            flexWrap: "wrap",
            gap: "1rem",
            boxShadow: "0 0 20px rgba(16, 185, 129, 0.2)"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.9rem" }}>
            <span
              style={{
                width: "14px",
                height: "14px",
                borderRadius: "50%",
                background: "#10b981",
                boxShadow: "0 0 12px #10b981",
                display: "inline-block",
                flexShrink: 0
              }}
            />
            <div>
              <strong style={{ color: "#ecfdf5", fontSize: "1.1rem" }}>
                ✅ Seu computador {onlineDevice.name || "conectado"} já está online e pronto para uso!
              </strong>
              <p style={{ margin: "0.25rem 0 0 0", color: "#a7f3d0", fontSize: "0.88rem" }}>
                O túnel seguro com o Remote Controller está ativo. A IA já pode ler seus projetos e executar tarefas locais.
              </p>
            </div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
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
                gap: "0.45rem",
                textDecoration: "none",
                borderRadius: "8px",
                padding: "0.55rem 1.1rem",
                fontSize: "0.88rem",
                border: "none",
                boxShadow: "0 4px 12px rgba(16, 185, 129, 0.3)"
              }}
            >
              Abrir ChatGPT agora <ExternalLink size={15} />
            </a>
          </div>
        </div>
      )}

      {/* CABEÇALHO DO ONBOARDING */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", flexWrap: "wrap", gap: "1.25rem" }}>
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem", marginBottom: "0.45rem", flexWrap: "wrap" }}>
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
              <ShieldCheck size={13} style={{ display: "inline", verticalAlign: "middle", marginRight: "4px" }} />
              Verificado SHA-256
            </span>
            {isCheckingStatus && (
              <span style={{ fontSize: "0.75rem", color: "#94a3b8", display: "inline-flex", alignItems: "center", gap: "4px" }}>
                <RefreshCw size={12} className="spin" /> Verificando conexão...
              </span>
            )}
          </div>
          <h2 style={{ fontSize: "1.75rem", margin: "0.25rem 0 0.5rem 0", color: "#f8fafc", fontWeight: 700, letterSpacing: "-0.02em" }}>
            Instalar no Windows em 1 Passo
          </h2>
          <p style={{ color: "#94a3b8", margin: 0, fontSize: "0.95rem", maxWidth: "700px", lineHeight: "1.6" }}>
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
            gap: "0.5rem",
            background: showDiagnostics ? "rgba(245, 158, 11, 0.1)" : "rgba(15, 23, 42, 0.6)",
            fontSize: "0.85rem",
            borderRadius: "8px",
            padding: "0.5rem 0.95rem"
          }}
        >
          <AlertCircle size={15} />
          {showDiagnostics ? "Ocultar Diagnóstico" : "Central de Diagnóstico & Dúvidas"}
        </button>
      </div>

      {/* JORNADA DO PRIMEIRO ACESSO (4 CARDS VISUAIS MODERNOS) */}
      <div style={{ display: "flex", flexDirection: "column", gap: "1.35rem", marginTop: "1.75rem" }}>
        
        {/* 🔹 PASSO 1: COPIAR O COMANDO DE PAREAMENTO SEGURO */}
        <div
          data-step="1"
          style={{
            background: "rgba(15, 23, 42, 0.85)",
            border: "1px solid #1e293b",
            borderRadius: "14px",
            padding: "1.5rem",
            position: "relative",
            backdropFilter: "blur(8px)"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: "0.6rem", marginBottom: "0.85rem" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "0.65rem" }}>
              <span
                style={{
                  background: "#10b981",
                  color: "#022c22",
                  width: "28px",
                  height: "28px",
                  borderRadius: "50%",
                  display: "inline-grid",
                  placeItems: "center",
                  fontWeight: 800,
                  fontSize: "0.9rem"
                }}
              >
                1
              </span>
              <strong style={{ fontSize: "1.15rem", color: "#f8fafc" }}>
                Passo 1: Copiar o Comando de Pareamento Seguro
              </strong>
            </div>

            <div style={{ display: "flex", gap: "0.45rem", flexWrap: "wrap" }}>
              <span style={{ fontSize: "0.75rem", background: "rgba(16, 185, 129, 0.12)", color: "#10b981", padding: "0.25rem 0.6rem", borderRadius: "6px", border: "1px solid rgba(16, 185, 129, 0.25)" }}>
                Isolado em %LOCALAPPDATA%
              </span>
              <span style={{ fontSize: "0.75rem", background: "rgba(56, 189, 248, 0.12)", color: "#38bdf8", padding: "0.25rem 0.6rem", borderRadius: "6px", border: "1px solid rgba(56, 189, 248, 0.25)" }}>
                Sem Admin
              </span>
            </div>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.92rem", margin: "0 0 1rem 0", lineHeight: "1.6" }}>
            Esse comando baixa e inicia o conector leve no seu computador, criando um túnel seguro e criptografado exclusivo da sua conta.
            O instalador <strong>não mexe no sistema global</strong> e instala isolado na pasta de usuário (<code>%LOCALAPPDATA%</code>),
            dispensando privilégios de Administrador.
          </p>

          <div
            style={{
              background: "#030712",
              border: "1px solid #1e293b",
              borderRadius: "10px",
              padding: "1rem 1.15rem",
              display: "flex",
              alignItems: "center",
              gap: "0.85rem",
              flexWrap: "wrap"
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.6rem", flex: 1, minWidth: "260px", overflowX: "auto" }}>
              <Terminal size={20} style={{ color: "#38bdf8", flexShrink: 0 }} />
              <code style={{ fontFamily: "Consolas, monospace", color: "#38bdf8", fontSize: "0.88rem", whiteSpace: "nowrap" }}>
                {OFFICIAL_INSTALL_CMD}
              </code>
            </div>

            <button
              type="button"
              onClick={() => copyToClipboard(OFFICIAL_INSTALL_CMD, setCopiedMain)}
              style={{
                flexShrink: 0,
                background: copiedMain ? "#10b981" : "#0284c7",
                color: copiedMain ? "#022c22" : "#ffffff",
                border: "none",
                padding: "0.65rem 1.35rem",
                borderRadius: "8px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.5rem",
                fontWeight: 700,
                fontSize: "0.92rem",
                transition: "all 0.2s",
                boxShadow: copiedMain ? "0 0 15px rgba(16, 185, 129, 0.35)" : "0 4px 12px rgba(2, 132, 199, 0.35)"
              }}
            >
              {copiedMain ? <Check size={16} /> : <Copy size={16} />}
              {copiedMain ? "Copiado!" : "Copiar Comando (1 Clique)"}
            </button>
          </div>
        </div>

        {/* 🔹 PASSO 2: EXECUTAR NO WINDOWS (SEM COMPLICAÇÃO) */}
        <div
          data-step="2"
          style={{
            background: "rgba(15, 23, 42, 0.85)",
            border: "1px solid #1e293b",
            borderRadius: "14px",
            padding: "1.5rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.65rem", marginBottom: "0.85rem" }}>
            <span
              style={{
                background: "#38bdf8",
                color: "#082f49",
                width: "28px",
                height: "28px",
                borderRadius: "50%",
                display: "inline-grid",
                placeItems: "center",
                fontWeight: 800,
                fontSize: "0.9rem"
              }}
            >
              2
            </span>
            <strong style={{ fontSize: "1.15rem", color: "#f8fafc" }}>
              Passo 2: Executar no Windows (Sem Complicação)
            </strong>
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(290px, 1fr))", gap: "1.35rem", alignItems: "start" }}>
            {/* Instruções mastigadas */}
            <div>
              <ol style={{ paddingLeft: "1.25rem", margin: 0, color: "#cbd5e1", fontSize: "0.92rem", lineHeight: "1.75" }}>
                <li>
                  Pressione a tecla <strong>Windows</strong> no teclado e digite <strong>PowerShell</strong>.
                </li>
                <li>
                  Clique com o botão esquerdo para abrir (<strong>NÃO precisa ser &quot;Executar como Administrador&quot;</strong>).
                </li>
                <li>
                  Cole o comando com <kbd style={{ background: "#1e293b", padding: "0.15rem 0.45rem", borderRadius: "4px", fontSize: "0.82rem", color: "#93c5fd" }}>Ctrl + V</kbd> (ou clique com o botão direito na janela azul/preta) e aperte <strong>Enter</strong>.
                </li>
              </ol>

              {/* Mini-mockup do Menu Iniciar */}
              <div
                style={{
                  marginTop: "1rem",
                  background: "#1e293b",
                  borderRadius: "10px",
                  padding: "0.85rem",
                  border: "1px solid #334155",
                  display: "flex",
                  alignItems: "center",
                  gap: "0.85rem"
                }}
              >
                <div style={{ width: "36px", height: "36px", background: "#0284c7", borderRadius: "8px", display: "grid", placeItems: "center", color: "#fff" }}>
                  <Terminal size={20} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: "0.88rem", color: "#f1f5f9", fontWeight: 600 }}>Windows PowerShell</div>
                  <div style={{ fontSize: "0.76rem", color: "#94a3b8" }}>Aplicativo · Pressione Enter para abrir</div>
                </div>
                <span style={{ fontSize: "0.75rem", color: "#10b981", background: "rgba(16, 185, 129, 0.15)", padding: "0.25rem 0.55rem", borderRadius: "4px" }}>
                  Sem Admin
                </span>
              </div>
            </div>

            {/* Preview visual do Terminal de Sucesso */}
            <div
              style={{
                background: "#020617",
                border: "1px solid #334155",
                borderRadius: "10px",
                overflow: "hidden"
              }}
            >
              <div style={{ background: "#0f172a", padding: "0.45rem 0.85rem", display: "flex", alignItems: "center", justifyContent: "space-between", borderBottom: "1px solid #1e293b" }}>
                <span style={{ fontSize: "0.75rem", color: "#94a3b8", fontFamily: "Consolas, monospace" }}>Windows PowerShell</span>
                <div style={{ display: "flex", gap: "6px" }}>
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#475569" }} />
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#475569" }} />
                  <span style={{ width: "9px", height: "9px", borderRadius: "50%", background: "#ef4444" }} />
                </div>
              </div>
              <div style={{ padding: "0.95rem", fontFamily: "Consolas, monospace", fontSize: "0.82rem", lineHeight: "1.65" }}>
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
            background: "rgba(15, 23, 42, 0.85)",
            border: "1px solid #1e293b",
            borderRadius: "14px",
            padding: "1.5rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: "0.6rem", marginBottom: "0.85rem" }}>
            <div style={{ display: "flex", alignItems: "center", gap: "0.65rem" }}>
              <span
                style={{
                  background: "#a855f7",
                  color: "#fff",
                  width: "28px",
                  height: "28px",
                  borderRadius: "50%",
                  display: "inline-grid",
                  placeItems: "center",
                  fontWeight: 800,
                  fontSize: "0.9rem"
                }}
              >
                3
              </span>
              <strong style={{ fontSize: "1.15rem", color: "#f8fafc" }}>
                Passo 3: Conectar e Autorizar no ChatGPT
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
                padding: "0.55rem 1.15rem",
                borderRadius: "8px",
                textDecoration: "none",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.45rem",
                fontSize: "0.9rem",
                boxShadow: "0 0 18px rgba(16, 185, 129, 0.3)"
              }}
            >
              Abrir o ChatGPT / Meu Agente <ExternalLink size={15} />
            </a>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.92rem", margin: "0 0 1rem 0", lineHeight: "1.6" }}>
            Adicione o endpoint MCP no ChatGPT e use a IA para ler e analisar seus projetos locais com segurança.
          </p>

          {/* BOX DE ALERTA DOURADO: ATENÇÃO AO BOTÃO PERMITIR */}
          <div
            style={{
              background: "linear-gradient(90deg, rgba(245, 158, 11, 0.15), rgba(217, 119, 6, 0.22))",
              border: "1px solid #f59e0b",
              borderRadius: "10px",
              padding: "1.1rem 1.35rem",
              marginTop: "0.85rem"
            }}
          >
            <div style={{ display: "flex", alignItems: "flex-start", gap: "0.85rem" }}>
              <AlertTriangle size={22} style={{ color: "#fbbf24", flexShrink: 0, marginTop: "2px" }} />
              <div>
                <strong style={{ color: "#fef3c7", fontSize: "1rem", display: "block", marginBottom: "0.4rem" }}>
                  PONTO CRÍTICO: Clique em &quot;Permitir&quot; (ou &quot;Always Allow&quot;) no ChatGPT
                </strong>
                <p style={{ margin: 0, color: "#fde68a", fontSize: "0.9rem", lineHeight: "1.6" }}>
                  Na primeira vez que você pedir algo, o ChatGPT exibirá uma janela de segurança perguntando se você autoriza a conexão.
                  Clique em <strong>Permitir</strong> (ou <strong>Sempre permitir</strong>). Se você fechar ou ignorar essa janela, a IA não conseguirá falar com o seu PC!
                </p>

                {/* Mockup visual da janela de autorização do ChatGPT */}
                <div
                  style={{
                    marginTop: "0.9rem",
                    background: "#0f172a",
                    border: "1px solid rgba(245, 158, 11, 0.4)",
                    borderRadius: "8px",
                    padding: "0.75rem 1rem",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    gap: "0.8rem",
                    flexWrap: "wrap"
                  }}
                >
                  <div style={{ fontSize: "0.85rem", color: "#f8fafc" }}>
                    <span>Janela ChatGPT: <em>&quot;Permitir que este app acesse seu computador?&quot;</em></span>
                  </div>
                  <div style={{ display: "flex", gap: "0.5rem" }}>
                    <span style={{ background: "#334155", color: "#94a3b8", padding: "0.3rem 0.65rem", borderRadius: "4px", fontSize: "0.78rem" }}>
                      Negar
                    </span>
                    <span
                      style={{
                        background: "#10b981",
                        color: "#022c22",
                        padding: "0.3rem 0.75rem",
                        borderRadius: "4px",
                        fontSize: "0.78rem",
                        fontWeight: 700,
                        border: "2px solid #34d399",
                        boxShadow: "0 0 10px rgba(52, 211, 153, 0.5)"
                      }}
                    >
                      👉 Permitir / Sempre permitir
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 🔹 PASSO 4: O TESTE DE BOAS-VINDAS */}
        <div
          data-step="4"
          style={{
            background: "rgba(15, 23, 42, 0.85)",
            border: "1px solid #1e293b",
            borderRadius: "14px",
            padding: "1.5rem"
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: "0.65rem", marginBottom: "0.85rem" }}>
            <span
              style={{
                background: "#f59e0b",
                color: "#451a03",
                width: "28px",
                height: "28px",
                borderRadius: "50%",
                display: "inline-grid",
                placeItems: "center",
                fontWeight: 800,
                fontSize: "0.9rem"
              }}
            >
              4
            </span>
            <strong style={{ fontSize: "1.15rem", color: "#f8fafc" }}>
              Passo 4: O Teste de Boas-Vindas
            </strong>
          </div>

          <p style={{ color: "#cbd5e1", fontSize: "0.92rem", margin: "0 0 0.95rem 0", lineHeight: "1.6" }}>
            Envie esta mensagem no ChatGPT para confirmar que o conector local está respondendo instantaneamente:
          </p>

          {/* Balão de sugestão de prompt */}
          <div
            style={{
              background: "#030712",
              border: "1px solid #22c55e",
              borderRadius: "10px",
              padding: "1rem 1.25rem",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "0.85rem",
              flexWrap: "wrap"
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.65rem", flex: 1, minWidth: "250px" }}>
              <Sparkles size={20} style={{ color: "#22c55e", flexShrink: 0 }} />
              <span style={{ color: "#f0fdf4", fontStyle: "italic", fontSize: "0.92rem" }}>
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
                padding: "0.45rem 0.95rem",
                borderRadius: "6px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.45rem",
                fontWeight: 600,
                fontSize: "0.85rem",
                transition: "all 0.2s"
              }}
            >
              {copiedPrompt ? <Check size={14} /> : <Copy size={14} />}
              {copiedPrompt ? "Copiado!" : "Copiar Exemplo"}
            </button>
          </div>

          {/* Dica Extra (Monitor Opcional): rdc-monitor.exe */}
          <div
            style={{
              marginTop: "1.1rem",
              padding: "0.95rem 1.15rem",
              background: "rgba(30, 41, 59, 0.5)",
              border: "1px solid #334155",
              borderRadius: "10px",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: "0.85rem",
              flexWrap: "wrap"
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "0.65rem", flex: 1, minWidth: "260px" }}>
              <Activity size={18} style={{ color: "#38bdf8", flexShrink: 0 }} />
              <div style={{ fontSize: "0.88rem", color: "#cbd5e1" }}>
                <strong>Dica Extra (Monitor Opcional):</strong> Deseja assistir na tela cada ação e diff de linha em tempo real? Abra o <code>rdc-monitor.exe</code>.
              </div>
            </div>
            <button
              type="button"
              onClick={() => copyToClipboard(POWERSHELL_MONITOR_CMD, setCopiedMonitor)}
              style={{
                background: copiedMonitor ? "#10b981" : "#1e293b",
                color: "#fff",
                border: "none",
                padding: "0.45rem 0.85rem",
                borderRadius: "6px",
                cursor: "pointer",
                display: "inline-flex",
                alignItems: "center",
                gap: "0.4rem",
                fontSize: "0.82rem",
                fontWeight: 600
              }}
            >
              {copiedMonitor ? <Check size={14} /> : <Copy size={14} />}
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
      <section
        style={{
          marginTop: "1.85rem",
          borderTop: "1px solid #1e293b",
          paddingTop: "1.65rem"
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "0.6rem", marginBottom: "0.85rem" }}>
          <AlertCircle size={22} style={{ color: "#f59e0b" }} />
          <h3 style={{ fontSize: "1.25rem", margin: 0, color: "#f1f5f9", fontWeight: 700 }}>
            Central de Diagnóstico &amp; Solução de Erros Comuns
          </h3>
        </div>
        <p style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: 0, marginBottom: "1.35rem" }}>
          Encontrou algum imprevisto? Clique no item abaixo para ver a solução imediata em 1 clique:
        </p>

        <div style={{ display: "flex", flexDirection: "column", gap: "0.85rem" }}>
          
          {/* 1. EXECUTION POLICY */}
          <div
            style={{
              background: "#0f172a",
              border: "1px solid #1e293b",
              borderRadius: "10px",
              overflow: "hidden"
            }}
          >
            <button
              type="button"
              onClick={() => toggleAccordion("policy")}
              style={{
                width: "100%",
                padding: "0.95rem 1.15rem",
                background: "transparent",
                border: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                color: "#f8fafc",
                fontWeight: 600,
                fontSize: "0.92rem",
                cursor: "pointer",
                textAlign: "left"
              }}
            >
              <span style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
                <span style={{ color: "#ef4444" }}>🔴</span>
                Erro: &quot;A execução de scripts foi desabilitada neste sistema (ExecutionPolicy)&quot;
              </span>
              {openAccordions["policy"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
            </button>

            {openAccordions["policy"] && (
              <div style={{ padding: "0.95rem 1.15rem 1.15rem 1.15rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                <p style={{ margin: "0 0 0.55rem 0", fontSize: "0.88rem", color: "#94a3b8" }}>
                  <strong>Causa:</strong> O Windows bloqueou scripts PowerShell por política de segurança padrão.
                </p>
                <p style={{ margin: "0 0 0.85rem 0", fontSize: "0.88rem", color: "#cbd5e1" }}>
                  <strong>Solução:</strong> O comando oficial já pode ser executado com o parâmetro inline <code>powershell -ExecutionPolicy Bypass -Command ...</code>, ou execute o comando alternativo em 1 clique abaixo:
                </p>
                
                <div style={{ display: "flex", flexDirection: "column", gap: "0.65rem" }}>
                  {/* Comando com Bypass inline */}
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.65rem 0.95rem", borderRadius: "8px", gap: "0.6rem", border: "1px solid #1e293b", flexWrap: "wrap" }}>
                    <code style={{ flex: 1, minWidth: "240px", color: "#38bdf8", fontSize: "0.84rem", whiteSpace: "nowrap", overflowX: "auto" }}>
                      {EXECUTION_POLICY_CMD}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard(EXECUTION_POLICY_CMD, setCopiedBypass)}
                      style={{
                        background: copiedBypass ? "#10b981" : "#1e293b",
                        color: "#fff",
                        border: "none",
                        padding: "0.45rem 0.85rem",
                        borderRadius: "6px",
                        fontSize: "0.82rem",
                        cursor: "pointer",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.35rem",
                        fontWeight: 600
                      }}
                    >
                      {copiedBypass ? <Check size={13} /> : <Copy size={13} />}
                      {copiedBypass ? "Copiado!" : "Copiar Comando Bypass"}
                    </button>
                  </div>

                  {/* Comando alternativo Set-ExecutionPolicy Process */}
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.65rem 0.95rem", borderRadius: "8px", gap: "0.6rem", border: "1px solid #1e293b", flexWrap: "wrap" }}>
                    <code style={{ flex: 1, minWidth: "240px", color: "#a5b4fc", fontSize: "0.84rem", whiteSpace: "nowrap", overflowX: "auto" }}>
                      {ALTERNATIVE_POLICY_CMD}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard(ALTERNATIVE_POLICY_CMD, setCopiedAltPolicy)}
                      style={{
                        background: copiedAltPolicy ? "#10b981" : "#1e293b",
                        color: "#fff",
                        border: "none",
                        padding: "0.45rem 0.85rem",
                        borderRadius: "6px",
                        fontSize: "0.82rem",
                        cursor: "pointer",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.35rem",
                        fontWeight: 600
                      }}
                    >
                      {copiedAltPolicy ? <Check size={13} /> : <Copy size={13} />}
                      {copiedAltPolicy ? "Copiado!" : "Copiar Alternativo"}
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* 2. CÓDIGO JÁ UTILIZADO (ALREADY USED) */}
          <div
            style={{
              background: "#0f172a",
              border: "1px solid #1e293b",
              borderRadius: "10px",
              overflow: "hidden"
            }}
          >
            <button
              type="button"
              onClick={() => toggleAccordion("code-used")}
              style={{
                width: "100%",
                padding: "0.95rem 1.15rem",
                background: "transparent",
                border: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                color: "#f8fafc",
                fontWeight: 600,
                fontSize: "0.92rem",
                cursor: "pointer",
                textAlign: "left"
              }}
            >
              <span style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
                <span style={{ color: "#ef4444" }}>🔴</span>
                Erro: &quot;Código de pareamento já utilizado (Already used)&quot;
              </span>
              {openAccordions["code-used"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
            </button>

            {openAccordions["code-used"] && (
              <div style={{ padding: "0.95rem 1.15rem 1.15rem 1.15rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                <p style={{ margin: "0 0 0.55rem 0", fontSize: "0.88rem", color: "#94a3b8" }}>
                  <strong>Causa:</strong> O código expirou (duração de 10 minutos) ou você tentou rodar o mesmo comando duas vezes em computadores diferentes.
                </p>
                <p style={{ margin: "0 0 0.85rem 0", fontSize: "0.88rem", color: "#cbd5e1" }}>
                  <strong>Solução:</strong> Gere um novo código de uso único instantâneo diretamente no botão abaixo ou reabra o agente no terminal:
                </p>

                <div style={{ display: "flex", flexDirection: "column", gap: "0.85rem" }}>
                  {/* BOTÃO DE GERAR NOVO CÓDIGO DIRETO NO SITE */}
                  <div style={{ display: "flex", alignItems: "center", gap: "0.75rem", flexWrap: "wrap" }}>
                    <button
                      type="button"
                      onClick={handleGenerateNewCode}
                      style={{
                        background: "#10b981",
                        color: "#022c22",
                        border: "none",
                        padding: "0.6rem 1.25rem",
                        borderRadius: "8px",
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "0.5rem",
                        fontSize: "0.9rem",
                        fontWeight: 700,
                        cursor: "pointer",
                        boxShadow: "0 0 15px rgba(16, 185, 129, 0.25)"
                      }}
                    >
                      <RefreshCw size={15} /> Gerar Novo Código de Pareamento
                    </button>
                    <a
                      href="/pair"
                      style={{
                        background: "#1e293b",
                        color: "#38bdf8",
                        border: "1px solid #334155",
                        padding: "0.55rem 1rem",
                        borderRadius: "8px",
                        display: "inline-flex",
                        alignItems: "center",
                        gap: "0.4rem",
                        fontSize: "0.88rem",
                        textDecoration: "none",
                        fontWeight: 600
                      }}
                    >
                      Ir para Página de Pareamento
                    </a>
                  </div>

                  {/* EXIBIÇÃO DO TOKEN GERADO INSTANTANEAMENTE */}
                  {generatedPairCode && (
                    <div
                      style={{
                        background: "rgba(16, 185, 129, 0.1)",
                        border: "1px solid #10b981",
                        borderRadius: "8px",
                        padding: "0.85rem 1.15rem",
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        gap: "0.75rem",
                        flexWrap: "wrap"
                      }}
                    >
                      <div>
                        <div style={{ fontSize: "0.78rem", color: "#a7f3d0", fontWeight: 700, letterSpacing: "0.05em" }}>
                          NOVO CÓDIGO GERADO COM SUCESSO (VÁLIDO POR 10 MINUTOS):
                        </div>
                        <div style={{ fontSize: "1.35rem", fontWeight: 800, color: "#ecfdf5", fontFamily: "Consolas, monospace", letterSpacing: "2px", marginTop: "2px" }}>
                          {generatedPairCode}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => copyToClipboard(generatedPairCode, setCopiedGeneratedCode)}
                        style={{
                          background: copiedGeneratedCode ? "#10b981" : "#1e293b",
                          color: "#fff",
                          border: "none",
                          padding: "0.45rem 0.95rem",
                          borderRadius: "6px",
                          fontSize: "0.85rem",
                          fontWeight: 600,
                          cursor: "pointer",
                          display: "inline-flex",
                          alignItems: "center",
                          gap: "0.4rem"
                        }}
                      >
                        {copiedGeneratedCode ? <Check size={14} /> : <Copy size={14} />}
                        {copiedGeneratedCode ? "Copiado!" : "Copiar Código"}
                      </button>
                    </div>
                  )}

                  {/* COMANDO DE PAREAMENTO NO TERMINAL */}
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.6rem 0.95rem", borderRadius: "8px", gap: "0.6rem", border: "1px solid #1e293b", flexWrap: "wrap" }}>
                    <code style={{ flex: 1, minWidth: "240px", color: "#38bdf8", fontSize: "0.84rem" }}>
                      &amp; &quot;$env:LOCALAPPDATA\Programs\RemoteDesktopController\device-agent.exe&quot; pair
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard('& "$env:LOCALAPPDATA\\Programs\\RemoteDesktopController\\device-agent.exe" pair', setCopiedPairCmd)}
                      style={{
                        background: copiedPairCmd ? "#10b981" : "#1e293b",
                        color: "#fff",
                        border: "none",
                        padding: "0.4rem 0.8rem",
                        borderRadius: "6px",
                        fontSize: "0.8rem",
                        cursor: "pointer",
                        display: "flex",
                        alignItems: "center",
                        gap: "0.35rem"
                      }}
                    >
                      {copiedPairCmd ? <Check size={13} /> : <Copy size={13} />}
                      {copiedPairCmd ? "Copiado!" : "Copiar Comando"}
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* 3. DISPOSITIVO OFFLINE */}
          <div
            style={{
              background: "#0f172a",
              border: "1px solid #1e293b",
              borderRadius: "10px",
              overflow: "hidden"
            }}
          >
            <button
              type="button"
              onClick={() => toggleAccordion("offline")}
              style={{
                width: "100%",
                padding: "0.95rem 1.15rem",
                background: "transparent",
                border: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                color: "#f8fafc",
                fontWeight: 600,
                fontSize: "0.92rem",
                cursor: "pointer",
                textAlign: "left"
              }}
            >
              <span style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
                <span style={{ color: "#ef4444" }}>🔴</span>
                Erro: &quot;O ChatGPT diz que o dispositivo está offline&quot;
              </span>
              {openAccordions["offline"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
            </button>

            {openAccordions["offline"] && (
              <div style={{ padding: "0.95rem 1.15rem 1.15rem 1.15rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                <p style={{ margin: "0 0 0.55rem 0", fontSize: "0.88rem", color: "#94a3b8" }}>
                  <strong>Causa:</strong> O terminal do agente foi fechado, o PC foi reiniciado sem o serviço ativo, ou o computador suspendeu a internet.
                </p>
                <p style={{ margin: "0 0 0.85rem 0", fontSize: "0.88rem", color: "#cbd5e1" }}>
                  <strong>Solução:</strong> Use o passo rápido de 1 linha para reabrir o conector ou checar se ele está rodando em segundo plano:
                </p>
                <div style={{ display: "flex", flexDirection: "column", gap: "0.65rem" }}>
                  {/* Reabrir conector */}
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.6rem 0.95rem", borderRadius: "8px", gap: "0.6rem", border: "1px solid #1e293b", flexWrap: "wrap" }}>
                    <code style={{ flex: 1, minWidth: "240px", color: "#38bdf8", fontSize: "0.84rem" }}>
                      {RESTART_AGENT_CMD}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard(RESTART_AGENT_CMD, setCopiedRestart)}
                      style={{ background: copiedRestart ? "#10b981" : "#1e293b", color: "#fff", border: "none", padding: "0.4rem 0.85rem", borderRadius: "6px", fontSize: "0.8rem", cursor: "pointer", display: "flex", alignItems: "center", gap: "0.35rem" }}
                    >
                      {copiedRestart ? <Check size={13} /> : <Copy size={13} />}
                      {copiedRestart ? "Copiado!" : "Copiar Comando de Reinício"}
                    </button>
                  </div>

                  {/* Checar status */}
                  <div style={{ display: "flex", alignItems: "center", background: "#030712", padding: "0.6rem 0.95rem", borderRadius: "8px", gap: "0.6rem", border: "1px solid #1e293b", flexWrap: "wrap" }}>
                    <code style={{ flex: 1, minWidth: "240px", color: "#a5b4fc", fontSize: "0.84rem" }}>
                      {STATUS_AGENT_CMD}
                    </code>
                    <button
                      type="button"
                      onClick={() => copyToClipboard(STATUS_AGENT_CMD, setCopiedStatus)}
                      style={{ background: copiedStatus ? "#10b981" : "#1e293b", color: "#fff", border: "none", padding: "0.4rem 0.85rem", borderRadius: "6px", fontSize: "0.8rem", cursor: "pointer", display: "flex", alignItems: "center", gap: "0.35rem" }}
                    >
                      {copiedStatus ? <Check size={13} /> : <Copy size={13} />}
                      {copiedStatus ? "Copiado!" : "Copiar Comando de Status"}
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
              borderRadius: "10px",
              overflow: "hidden"
            }}
          >
            <button
              type="button"
              onClick={() => toggleAccordion("smartscreen")}
              style={{
                width: "100%",
                padding: "0.95rem 1.15rem",
                background: "transparent",
                border: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                color: "#f8fafc",
                fontWeight: 600,
                fontSize: "0.92rem",
                cursor: "pointer",
                textAlign: "left"
              }}
            >
              <span style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
                <span style={{ color: "#ef4444" }}>🔴</span>
                Erro: &quot;Aviso do Windows Defender / SmartScreen&quot;
              </span>
              {openAccordions["smartscreen"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
            </button>

            {openAccordions["smartscreen"] && (
              <div style={{ padding: "0.95rem 1.15rem 1.15rem 1.15rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                <p style={{ margin: "0 0 0.55rem 0", fontSize: "0.88rem", color: "#94a3b8" }}>
                  <strong>Causa:</strong> O executável é novo e o Windows não tem histórico prévio de telemetria.
                </p>
                <p style={{ margin: "0 0 0.85rem 0", fontSize: "0.88rem", color: "#cbd5e1" }}>
                  <strong>Solução:</strong>
                </p>
                
                {/* Ilustração visual passo a passo */}
                <div
                  style={{
                    background: "#0284c7",
                    borderRadius: "10px",
                    padding: "1rem 1.25rem",
                    color: "#ffffff",
                    marginBottom: "0.85rem",
                    boxShadow: "0 4px 15px rgba(2, 132, 199, 0.25)"
                  }}
                >
                  <div style={{ fontSize: "0.9rem", fontWeight: 700, marginBottom: "0.45rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
                    <ShieldCheck size={18} /> O Windows protegeu o seu computador (SmartScreen)
                  </div>
                  <div style={{ fontSize: "0.82rem", opacity: 0.9, marginBottom: "0.85rem" }}>
                    O Microsoft Defender SmartScreen impediu a inicialização de um aplicativo não reconhecido.
                  </div>
                  <div style={{ display: "flex", gap: "0.75rem", alignItems: "center", flexWrap: "wrap" }}>
                    <span
                      style={{
                        background: "rgba(255, 255, 255, 0.2)",
                        border: "1px solid #ffffff",
                        padding: "0.35rem 0.85rem",
                        borderRadius: "6px",
                        fontSize: "0.84rem",
                        fontWeight: 700,
                        cursor: "default"
                      }}
                    >
                      1º Clique em: &quot;Mais informações&quot;
                    </span>
                    <span style={{ fontSize: "0.9rem" }}>➔</span>
                    <span
                      style={{
                        background: "#ffffff",
                        color: "#0369a1",
                        padding: "0.35rem 0.85rem",
                        borderRadius: "6px",
                        fontSize: "0.84rem",
                        fontWeight: 800,
                        boxShadow: "0 2px 8px rgba(0,0,0,0.2)"
                      }}
                    >
                      2º Clique em: &quot;Executar assim mesmo&quot;
                    </span>
                  </div>
                </div>

                <ol style={{ paddingLeft: "1.25rem", margin: 0, color: "#cbd5e1", fontSize: "0.88rem", lineHeight: "1.7" }}>
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
              borderRadius: "10px",
              overflow: "hidden"
            }}
          >
            <button
              type="button"
              onClick={() => toggleAccordion("timeout")}
              style={{
                width: "100%",
                padding: "0.95rem 1.15rem",
                background: "transparent",
                border: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                color: "#f8fafc",
                fontWeight: 600,
                fontSize: "0.92rem",
                cursor: "pointer",
                textAlign: "left"
              }}
            >
              <span style={{ display: "flex", alignItems: "center", gap: "0.6rem" }}>
                <span style={{ color: "#ef4444" }}>🔴</span>
                Erro: &quot;O ChatGPT fica carregando infinitamente e dá erro de timeout&quot;
              </span>
              {openAccordions["timeout"] ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
            </button>

            {openAccordions["timeout"] && (
              <div style={{ padding: "0.95rem 1.15rem 1.15rem 1.15rem", borderTop: "1px solid #1e293b", background: "#0b0f19" }}>
                <p style={{ margin: "0 0 0.55rem 0", fontSize: "0.88rem", color: "#94a3b8" }}>
                  <strong>Causa:</strong> O usuário não clicou no botão &quot;Permitir&quot; da janela modal de autorização do ChatGPT.
                </p>
                <p style={{ margin: "0 0 0.85rem 0", fontSize: "0.88rem", color: "#cbd5e1" }}>
                  <strong>Solução:</strong> Atualize a página do ChatGPT (<kbd style={{ background: "#1e293b", padding: "0.15rem 0.4rem", borderRadius: "4px" }}>F5</kbd>) e repita a requisição. Assim que o balão aparecer logo acima da caixa de resposta, aceite clicando em <strong>&quot;Permitir&quot;</strong> ou <strong>&quot;Sempre permitir&quot;</strong>.
                </p>
                
                {/* Dica visual */}
                <div
                  style={{
                    background: "rgba(16, 185, 129, 0.1)",
                    border: "1px solid #10b981",
                    borderRadius: "8px",
                    padding: "0.85rem 1rem",
                    display: "flex",
                    alignItems: "center",
                    gap: "0.75rem"
                  }}
                >
                  <CheckCircle2 size={20} style={{ color: "#10b981", flexShrink: 0 }} />
                  <span style={{ fontSize: "0.85rem", color: "#d1fae5" }}>
                    Dica: Ao selecionar <strong>&quot;Sempre permitir&quot;</strong>, o ChatGPT não solicitará confirmação manual a cada comando, tornando as respostas instantâneas.
                  </span>
                </div>
              </div>
            )}
          </div>

        </div>
      </section>
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
