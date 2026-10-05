# Remote Desktop Controller — plano de conclusão

> Execução por workers com fronteiras independentes e revisão integrada. Usar contratos do spec e ciclos RED/GREEN para funcionalidades de risco; commits pequenos após validação.

**Goal:** entregar o produto remoto preservando o runtime local.
**Architecture:** serviço Rust com contas/OAuth/MCP; agente Rust com saída HTTPS para relay e IPC autenticado local; histórico persistente por sessão.
**Tech Stack:** Rust/Tokio, Axum, reqwest/rustls, libSQL, Moka, Argon2, rmcp/protocolo MCP.
**Spec:** docs/product/spec.md

## Constraints

- Preservar dados, compatibilidade v0.2 e política de execução. Sem deploy fictício.
- Duas frentes auxiliares; root integra, executa builds seriais e revisa. Nenhum worker cria outros agentes.
- CPU Cargo jobs=2; reutilizar target existente. Não executar builds globais concorrentes.
- Código e contratos seguem spec; incertezas resolvidas autonomamente e registradas.

## Task 01 — baseline, contratos e ledger
- [x] Conferir base, documentação e autorizações.
- [x] Criar worktree gerenciado e branch `codex/remote-product`.
- [x] Registrar spec, contratos HTTP e critérios.
- [x] Rodar baseline e registrar gates do código original.

## Task 02 — serviço, contas, OAuth, pareamento e MCP
Arquivos: apps/remote-service/**, docs/product/service-report.md; root integra Cargo.toml.
- [x] Testes de PKCE, tokens, pairing, isolation e HTTP.
- [x] Implementar persistência, API, UI de conta/dispositivos e MCP.
- [x] Validar fluxos e revisar contrato/security; reportar evidências.

## Task 03 — sessões, logs e recuperação local
Arquivos: crates/domain, ports, application, protocol, infrastructure; migrations/002_*; docs/product/runtime-report.md.
- [x] Testes de migration/restart/isolation/log offsets/search.
- [x] Implementar sessões, propagação, spool consultável e busca histórica.
- [x] Validar focused tests e preservar gates existentes.

## Task 04 — agente remoto e conexão ao runtime
Arquivos: apps/device-agent/**, apps/mcp-gateway/src/main.rs.
- [x] Testar pareamento/poll/retry/operation identity e credenciais privadas.
- [x] Implementar CLI simples, reconexão, heartbeat, IPC e diagnóstico.
- [x] Testar agente real + daemon + relay sobre HTTP loopback.

## Task 05 — completar arquivos e terminal
Arquivos: crates/infrastructure/src/terminal*, files*, tools*; novos testes.
- [x] Implementar funções ausentes com confinamento e auditoria.
- [x] Implementar stdin e PTY/ConPTY com concessão local explícita.
- [x] Validar conflitos, cancelamento, quotas, restart e isolamento.

## Task 06 — onboarding, execução persistente e distribuição
Arquivos: apps/device-agent, tools, packaging, plugin, deployment.
- [x] Fluxo instalar/parear/iniciar/parar/revogar e autostart reversível.
- [x] Container/persistência/HTTPS, scripts de implantação e health checks.
- [x] Plugin com manifesto/skills e artefatos de revisão compatíveis.
- [x] Pacotes, checksums, UI e documentação verificados.

## Task 07 — revisão, integração e gates finais
- [x] Revisão independente proporcional ao risco; corrigir findings reais.
- [x] fmt/check/Clippy/build/test/benchmark focado; CI tri-OS.
- [x] Cenários reais de duas contas, devices, retry, revoke, restart.
- [x] Registrar resultados, limitações e percentual técnico ponderado.

## Task 08 — trabalho manual do proprietário (sempre por último)
- [ ] Executar somente ações que exigem conta, credencial ou titularidade indisponível.
- [ ] Confirmar hostname/HTTPS e OAuth redirect da plataforma.
- [ ] Assinar/publicar e aprovar políticas conforme identidade real.
- [ ] Conectar conta ChatGPT, realizar teste guiado e enviar confirmação.

## Rulings

- Autorização ampla do usuário substitui gates de perguntas adicionais sobre decisões reversíveis. Custos/credenciais/titularidade continuam explícitos ao final.
- Workers independentes editam superfícies separadas. Manifesto/lock e builds são centralizados para evitar concorrência e desperdício.
- Relatórios mantêm referências ao código/testes; não contar scaffold como entrega concluída.

## Evidência corrente

Implementação e regressões locais estão em validation.md; CI 37305576185 já aprovou container e todos os gates Linux. Checkboxes de release/distribuição foram fechados após os gates finais registrados abaixo. O histórico de requisitos/limitações é preservado no spec e security.md.

Gates finais completos: Windows 37313011274 e Linux/macOS/container 37309996209. Artefatos finais e manual do proprietário prontos; status.json = 99 técnicos / público false.
