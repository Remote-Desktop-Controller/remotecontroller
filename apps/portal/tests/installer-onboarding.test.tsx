import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { WindowsInstallerCard, McpChatGPTTutorial, OFFICIAL_INSTALL_CMD, OFFICIAL_MCP_URL } from "../src/components/windows-installer";

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
