import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  WindowsInstallerCard,
  McpChatGPTTutorial,
  OFFICIAL_INSTALL_CMD,
  OFFICIAL_MCP_URL,
  EXECUTION_POLICY_CMD,
  TEST_PROMPT_EXAMPLE,
  RESTART_AGENT_CMD,
  MONITOR_CMD
} from "../src/components/windows-installer";

describe("Windows Installer & Onboarding Guide", () => {
  it("renders Windows installer card with official PowerShell command and SHA-256 notice", () => {
    const html = renderToStaticMarkup(<WindowsInstallerCard />);
    expect(html).toContain("Instalar no Windows em 1 Passo");
    expect(html).toContain("Verificado SHA-256");
    expect(html.replace(/&quot;/g, '"').replace(/&amp;/g, '&')).toContain(OFFICIAL_INSTALL_CMD);
    expect(html).toContain("Copiar comando");
    expect(html).toContain("1. Cole no PowerShell");
    expect(html).toContain("2. Pareie com 1 Clique");
    expect(html).toContain("3. Conecte ao ChatGPT");
  });

  it("renders MCP connection guide with official URL and safety notice", () => {
    const html = renderToStaticMarkup(<McpChatGPTTutorial />);
    expect(html).toContain("Tutorial: Conectar como Servidor MCP no ChatGPT");
    expect(html).toContain(OFFICIAL_MCP_URL);
    expect(html).toContain("Connected Apps");
    expect(html).toContain("OAuth");
    expect(html).toContain("somente leitura");
  });

  it("renders dynamic online banner when machine is active", () => {
    const html = renderToStaticMarkup(
      <WindowsInstallerCard
        initialDevice={{
          device_id: "dev-test-1",
          name: "Workstation-Alpha",
          online: true
        }}
      />
    );
    expect(html).toContain("✅ Seu computador Workstation-Alpha já está online e pronto para uso!");
    expect(html).toContain("Abrir ChatGPT agora");
  });

  it("renders all 4 visual onboarding journey steps with detailed instructions and previews", () => {
    const html = renderToStaticMarkup(<WindowsInstallerCard />);

    // Passo 1
    expect(html).toContain("1. Cole no PowerShell (Copiar Comando Seguro)");
    expect(html).toContain("túnel seguro e criptografado exclusivo da sua conta");
    expect(html).toContain("não mexe no sistema global");
    expect(html).toContain("%LOCALAPPDATA%");
    expect(html).toContain("Sem Admin");

    // Passo 2
    expect(html).toContain("2. Pareie com 1 Clique (Executar no Windows)");
    expect(html).toContain("Pressione a tecla");
    expect(html).toContain("NÃO precisa ser &quot;Executar como Administrador&quot;");
    expect(html).toContain("[OK] Instalado com sucesso!");
    expect(html).toContain("[OK] Computador Pareado com Sucesso!");

    // Passo 3
    expect(html).toContain("3. Conecte ao ChatGPT (Abrir Meu Agente)");
    expect(html).toContain("Abrir o ChatGPT / Meu Agente");
    expect(html).toContain("PONTO CRÍTICO: Clique em &quot;Permitir&quot; (ou &quot;Always Allow&quot;) no ChatGPT");
    expect(html).toContain("Se você fechar ou ignorar essa janela, a IA não conseguirá falar com o seu PC!");

    // Passo 4
    expect(html).toContain("4. O Teste de Boas-Vindas &amp; Monitor em Tempo Real");
    expect(html).toContain(TEST_PROMPT_EXAMPLE);
    expect(html).toContain("Copiar Exemplo");
    expect(html).toContain("rdc-monitor.exe");
    expect(html).toContain("Copiar comando rdc-monitor");
  });

  it("renders the 5 diagnostic scenarios in Central de Diagnóstico accordion", () => {
    const html = renderToStaticMarkup(<WindowsInstallerCard />);

    expect(html).toContain("Central de Diagnóstico &amp; Solução de Erros Comuns");

    // Erro 1: ExecutionPolicy
    expect(html).toContain("A execução de scripts foi desabilitada neste sistema (ExecutionPolicy)");
    expect(html.replace(/&quot;/g, '"').replace(/&amp;/g, '&')).toContain(EXECUTION_POLICY_CMD);

    // Erro 2: Código já utilizado / expirado
    expect(html).toContain("Código de pareamento já utilizado (Already used) ou expirado");
    expect(html).toContain("Ir para Página de Pareamento");

    // Erro 3: Dispositivo offline
    expect(html).toContain("O ChatGPT diz que o dispositivo está offline");
    expect(html.replace(/&quot;/g, '"').replace(/&amp;/g, '&')).toContain(RESTART_AGENT_CMD);

    // Erro 4: Windows Defender / SmartScreen
    expect(html).toContain("Aviso do Windows Defender / SmartScreen");
    expect(html).toContain("Mais informações");
    expect(html).toContain("Executar assim mesmo");

    // Erro 5: Timeout / Carregando infinitamente no ChatGPT
    expect(html).toContain("O ChatGPT fica carregando infinitamente e dá erro de timeout");
    expect(html).toContain("Atualize a página do ChatGPT");
    expect(html).toContain("Permitir");
  });
});
