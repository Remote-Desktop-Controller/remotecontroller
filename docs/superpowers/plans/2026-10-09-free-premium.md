# Free + Premium Implementation Plan

**Goal:** implementar a especificação integral de Free + Premium autorizada no anexo.
**Architecture:** ledger de intervalos PostgreSQL, API autenticada Next e despacho Rust com prazo limitado; UI consulta somente estado.
**Spec:** docs/superpowers/specs/2026-10-09-free-premium.md
**Global constraints:** SALES_ENABLED=false; não executar migrations produção/reiniciar VPS/alterar gate; preço fixo 990; prazo 2592000s; quota 1800s; America/Sao_Paulo.

## Task 1 — quota PostgreSQL
- [x] Escrever testes PGlite comportamentais; observar falha com RPC ausente.
- [x] Criar migration aditiva 202610090003_daily_free.sql, RPC portal_usage e portal_usage_status; RLS e privilégios.
- [x] Verificar consumo, concorrência, replay, virada de dia, planos e recuperação conservadora.

## Task 2 — despacho Rust/agente
- [ ] Escrever testes de contrato e bypass antes de alterações.
- [ ] Integrar /api/mcp/usage na autorização, reserva no poll e conclusão no result; persistir prazo; aplicar política de operações não delimitadas.
- [ ] Testar remote-service/device-agent/daemon, deadline e erros estruturados.

## Task 3 — portal e interface
- [x] Testar Billing/handlers plano Free sem Asaas e preço fixo.
- [x] Implementar Billing.usage/usageStatus, endpoint autenticado e dashboard com saldo, barra, reset, Premium.
- [ ] Verificar typecheck, lint, build e suíte integrada sem cobranças reais.

## Task 4 — revisão e publicação
- [ ] Revisão independente de segurança e correção dos achados.
- [ ] Commit/push branch e publicar portal seguro, gate desativado.
- [ ] Documentar migrations pendentes, evidências, rollback e pedir autorização concreta para ativação.

## Ledger
Ruling: o anexo é especificação e autorização de implementação; nenhuma nova aprovação de design é necessária.
Ruling: checkout novo baseado em feat/commercial-portal; copiar cinco arquivos locais de autorização administrativa/reconciliação sem alterar origem; registrar como pré-requisito.
Task interfaces: 1 produz RPCs definidos na spec, 2 consome HTTP definido na spec, 3 adapta HTTP ao SQL. Cada executor edita arquivos próprios.

Task 1: complete — 24/24 testes SQL; privilégios, replay legado e Premium pendente revisados.
Ruling: endpoint legado authorize é negado quando Free habilitado, para versões antigas não despacharem sem quota.
Ruling: inspect Premium pendente verificado indica elegibilidade, status indica plano ativo; ativação somente no reserve.
Review round 1: quatro P1 Rust/Premium; Premium corrigido, três runtime em correção com testes.
Review round 2: SQL/portal sem bloqueantes; teste HTTP Free passa e build portal passou.

Review round 3: SQL32tests passa após isenção por multirange histórico; Premium revogado/Admin expirado retomam Free. Abort/claim e orçamentoSQLrelativo implementados, redelivery metadata imutável em teste Rust.
