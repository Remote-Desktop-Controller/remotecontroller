# Commercial Portal Implementation Plan

> Execute autonomously per the user's specification. Use subagent-driven-development for independent tasks and requesting-code-review for task and final reviews.

**Goal:** entregar portal pré-pago independente da VPS, com identidade comercial verificável e ativação apenas no primeiro uso remoto autorizado.
**Architecture:** Next/Supabase/Asaas como autoridade comercial; Rust/libSQL continua operacional e consulta APIs autenticadas do portal. Interface mobile em português, configuração comercial e cobrança real bloqueadas por padrão.
**Spec:** `docs/commercial-portal/spec.md` e pedido anexado do responsável.
**Constraints:** branch `feat/commercial-portal`; nenhum DNS/blog/main modificado; sem renovação; sem segredos; exactly 2592000 segundos por direito; não publicar pagamentos reais ou habilitar gate na VPS prematuramente.

## Task 1 — estrutura e contratos
- [ ] Criar `apps/portal/package.json`, TypeScript/Next/Tailwind/Vitest configs e exemplos de ambiente.
- [ ] Definir contratos HTTP/identidade e ledger; registrar decisões de disponibilidade/suporte.

## Task 2 — interface
- [ ] Implementar landing, auth, checkout, dashboard, compra/ativação, histórico, instalação, termos, privacidade, admin e `/connect` em `apps/portal/src/app` e `src/components`.
- [ ] Consumir apenas contratos `spec.md`; nunca criar dados pagos fictícios em produção ou iniciar período por login.
- [ ] Validar responsividade mobile e estados vazios/configuração faltante; guardar screenshots.

## Task 3 — autoridade comercial
- [ ] Implementar `apps/portal/src/lib`, `src/app/api`, `src/app/auth/callback` e `supabase/migrations`.
- [ ] Criar migração PostgreSQL com RLS, locks por usuário, pedidos/preço snapshot, direitos, períodos, eventos, tickets de identidade e rate limiting distribuído.
- [ ] Escrever testes de pré-venda, pagamento confirmado/duplicado/não confirmado, expiração/reembolso, ativação concorrente/retries, RLS e isolamento; executar antes/depois.
- [ ] Checkout real avulso PIX; reserva antes de I/O, recuperação por externalReference e nenhum retry POST ambíguo.
- [ ] APIs MCP com ticket de sessão verificável e autorização atômica, sem ativar por visita ao portal/OAuth.

## Task 4 — serviço remoto
- [ ] Implementar `apps/remote-service/src/commercial.rs`, config/rotas/migração aditiva, vínculo de sessão comercial e gates no relay.
- [ ] Escrever testes HTTP isolados de nonce/ticket, identidade, token antigo, bypass, serviço não disponível, primeira operação e expiração.
- [ ] Preservar e executar suites existentes remote-service/device-agent. Adicionar somente flags opcionais de implantação.

## Task 5 — integração, review e entrega
- [ ] Revisar contratos e executar suites, typecheck, build Next, lint/checks Rust, UI QA e SQL.
- [ ] Criar guia Supabase/Auth SMTP/Asaas webhook/API/preço/Vercel Root Directory/checkout retorno/MCP futuro, sem DNS/blog.
- [ ] Revisão independente das tarefas e whole-branch; corrigir defects importantes e repetir checks afetados.
- [ ] Fazer commits apenas nesta branch; publicar branch se permitido pelo acesso Git e informar URL/SHAs. Não merge/publish de site sem credenciais/condições comerciais.
- [ ] Relatório distingue implementado/testado localmente de provas Supabase/Asaas/Vercel/VPS externas.
