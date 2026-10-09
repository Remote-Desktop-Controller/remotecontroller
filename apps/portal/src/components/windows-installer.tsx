"use client";
import React, { useState } from "react";
import { Copy, Check, Terminal, ShieldCheck, Cpu, HelpCircle, ChevronDown, ChevronUp } from "lucide-react";

export const OFFICIAL_INSTALL_CMD = "irm https://raw.githubusercontent.com/Remote-Desktop-Controller/remotecontroller-windows/main/install.ps1 | iex";
export const OFFICIAL_MCP_URL = "https://andredozero.webajato.com.br/mcp";

export function WindowsInstallerCard() {
  const [copied, setCopied] = useState(false);
  const [showTutorial, setShowTutorial] = useState(false);

  async function copyCommand() {
    try {
      if (typeof navigator !== "undefined" && navigator.clipboard) {
        await navigator.clipboard.writeText(OFFICIAL_INSTALL_CMD);
        setCopied(true);
        setTimeout(() => setCopied(false), 3000);
      }
    } catch {}
  }

  return (
    <section className="surface-card installer-card" style={{ marginTop: "1.5rem", border: "1px solid var(--border-color, #2d3748)" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", flexWrap: "wrap", gap: "1rem" }}>
        <div>
          <p className="eyebrow" style={{ color: "#10b981", fontWeight: 700, margin: 0, letterSpacing: "0.05em" }}>INSTALAÇÃO GUIADA WINDOWS</p>
          <h2 style={{ fontSize: "1.5rem", margin: "0.25rem 0 0.5rem 0", color: "#f8fafc" }}>Instalar no Windows em 1 Passo</h2>
          <p style={{ color: "#94a3b8", margin: 0, fontSize: "0.95rem" }}>
            Instalação oficial sem compilação, sem arquivos ZIP manuais e com validação criptográfica SHA-256.
          </p>
        </div>
        <span className="pill" style={{ background: "rgba(16, 185, 129, 0.15)", color: "#10b981", padding: "0.35rem 0.75rem", borderRadius: "9999px", display: "inline-flex", alignItems: "center", gap: "0.35rem", fontSize: "0.85rem" }}>
          <ShieldCheck size={16} /> Verificado SHA-256
        </span>
      </div>

      <div style={{ marginTop: "1.25rem" }}>
        <label style={{ display: "block", fontSize: "0.85rem", color: "#cbd5e1", marginBottom: "0.5rem", fontWeight: 600 }}>
          Copie e execute no PowerShell do seu Windows:
        </label>
        <div style={{ display: "flex", alignItems: "center", background: "#090d16", padding: "0.75rem 1rem", borderRadius: "8px", border: "1px solid #1e293b", gap: "0.75rem", overflowX: "auto" }}>
          <Terminal size={18} style={{ color: "#38bdf8", flexShrink: 0 }} />
          <code style={{ flex: 1, fontFamily: "monospace", color: "#38bdf8", fontSize: "0.9rem", whiteSpace: "nowrap" }}>
            {OFFICIAL_INSTALL_CMD}
          </code>
          <button
            type="button"
            className="button button-small"
            onClick={copyCommand}
            style={{
              flexShrink: 0,
              background: copied ? "#10b981" : "#1e293b",
              color: "#fff",
              border: "none",
              padding: "0.4rem 0.85rem",
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
            {copied ? <Check size={14} /> : <Copy size={14} />}
            {copied ? "Copiado!" : "Copiar comando"}
          </button>
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: "1rem", marginTop: "1.5rem" }}>
        <div style={{ background: "rgba(255,255,255,0.02)", padding: "1rem", borderRadius: "8px", border: "1px solid rgba(255,255,255,0.05)" }}>
          <span style={{ color: "#38bdf8", fontWeight: 800, fontSize: "1.1rem" }}>1. Cole no PowerShell</span>
          <p style={{ margin: "0.35rem 0 0 0", fontSize: "0.85rem", color: "#94a3b8" }}>
            Abra o PowerShell (não requer Administrador) e pressione Enter. O pacote oficial é baixado e verificado.
          </p>
        </div>
        <div style={{ background: "rgba(255,255,255,0.02)", padding: "1rem", borderRadius: "8px", border: "1px solid rgba(255,255,255,0.05)" }}>
          <span style={{ color: "#38bdf8", fontWeight: 800, fontSize: "1.1rem" }}>2. Pareie com 1 Clique</span>
          <p style={{ margin: "0.35rem 0 0 0", fontSize: "0.85rem", color: "#94a3b8" }}>
            O instalador gera um código de uso único e abre automaticamente seu navegador para autorizar o computador.
          </p>
        </div>
        <div style={{ background: "rgba(255,255,255,0.02)", padding: "1rem", borderRadius: "8px", border: "1px solid rgba(255,255,255,0.05)" }}>
          <span style={{ color: "#38bdf8", fontWeight: 800, fontSize: "1.1rem" }}>3. Conecte ao ChatGPT</span>
          <p style={{ margin: "0.35rem 0 0 0", fontSize: "0.85rem", color: "#94a3b8" }}>
            Adicione o endpoint MCP no ChatGPT e use a IA para ler e analisar seus projetos locais com segurança.
          </p>
        </div>
      </div>

      <div style={{ marginTop: "1.25rem", borderTop: "1px solid rgba(255,255,255,0.06)", paddingTop: "1rem" }}>
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
            fontSize: "0.95rem",
            padding: 0
          }}
        >
          <HelpCircle size={18} />
          {showTutorial ? "Ocultar tutorial de conexão MCP ao ChatGPT" : "Como conectar o Remote Controller ao ChatGPT (MCP)"}
          {showTutorial ? <ChevronUp size={16} /> : <ChevronDown size={16} />}
        </button>

        {showTutorial && <McpChatGPTTutorial />}
      </div>
    </section>
  );
}

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
