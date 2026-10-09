# Validação comercial integrada — 2026-10-09

## Alterações

A autorização comercial reconcilia os pedidos elegíveis da própria conta com GET no Asaas antes de permitir operações. Apenas pagamentos PIX RECEIVED, com identidade/valor/referência conferidos e sem estorno DONE, entram na lista de pedidos verificados. O RPC recebe essa lista sob o mesmo bloqueio transacional por usuário: créditos inseridos simultaneamente, mas ainda não verificados, não podem iniciar um período. Falhas de consulta impedem novas autorizações sem iniciar dias. Consultas externas continuam fora das transações.

Estornos totais/qualquer parcial concluído revogam o respectivo direito. Não há revogação por mera existência de um pedido de estorno pendente. Um estado financeiro não confirmado impede autorizações até conciliação. A revogação é permanente para aquele direito; um evento RECEIVED atrasado não o recupera. Outros créditos legítimos da mesma conta continuam independentes.

A documentação oficial lista PAYMENT_PARTIALLY_REFUNDED, mas sua ausência no painel não pode ser a única proteção. A autorização consulta também refunds[].status=DONE; o diagnóstico exige apenas os quatro eventos essenciais disponíveis. Fontes: https://docs.asaas.com/docs/webhook-para-cobrancas e https://docs.asaas.com/docs/estornos.

O agente remoto mantém o long-poll ocioso sem consultas comerciais a cada 250 ms. Autorizações comerciais são verificadas antes da entrega de comandos pendentes; negações comerciais usam resposta recuperável, preservando a conexão do agente e a conclusão de operações previamente admitidas. Revogação local de dispositivo permanece separada.

## Migration obrigatória antes de habilitar a integração

Aplicar, nesta ordem, no Supabase EXCLUSIVO do portal:

1. `apps/portal/supabase/migrations/202610080001_commercial.sql` — já aplicada anteriormente; não reaplicar.
2. `apps/portal/supabase/migrations/202610090001_verified_authorization.sql` — nova; executar integralmente no SQL Editor ou pelo procedimento de migrations autorizado.

A nova migration substitui somente a função portal_authorize e seus privilégios, dentro de transação; não elimina tabelas nem dados. O quinto argumento p_verified_orders restringe a seleção dos direitos às cobranças reconciliadas. Chamadas HTTP sempre passam a lista, inclusive vazia. Compatibilidade de quatro argumentos existe apenas para os chamadores administrativos service_role; anon/authenticated não recebem EXECUTE.

Sem aplicar a migration, o novo endpoint de autorização falha fechado; não habilitar REMOTE_SERVICE_AVAILABLE. A publicação do portal com SALES_ENABLED=false, serviço indisponível e sem credenciais MCP não ativa esse caminho.

## Infraestrutura real e limites da prova

GET público da VPS comprovou /healthz 200, discovery OAuth 200 com issuer HTTPS correto e S256, protected-resource 200 apontando /mcp, /login 200 e /mcp sem token 401. A tela atual continua no login legado. Isso não comprova a licença comercial, o armazenamento operacional ou sessões de clientes reais.

Os testes financeiros não criam cobranças reais. O servidor Asaas local é simulado. Auth local é fixture: cadastro/login testam handlers e regras de aplicação, não entrega SMTP, verificação de e-mail ou sessão hospedada do Supabase. PGlite executa SQL PostgreSQL real em conexão isolada; não representa contenção distribuída no Supabase. O adaptador PostgREST local não substitui uma prova do filtro no PostgREST hospedado.

Configuração de webhook e requisições feitas pelo operador não comprovam entrega originada pelo Asaas. Para a prova hospedada, usar sandbox separado ou um evento legítimo existente, correlacionando ID no provedor, requisição na Vercel, evento SQL e direito único. Não trocar segredos/ambiente de produção nem gerar dinheiro real para obter evidência.

## Preservar e conectar a VPS — ainda exige acesso e autorização

Não publicar Rust nem ativar as variáveis comerciais na VPS nesta etapa. Para preparação operacional são necessários: host/usuário SSH autorizado e chave já instalada localmente (nunca no chat), diretório real do compose e dos dados libSQL, imagem/revisão implantada e inventário de usuários/dispositivos/sessões. Obter snapshot consistente por backup API ou VACUUM INTO, verificar integridade e testar restauração isolada antes da mudança; não copiar banco vivo em WAL nem recriar volumes.

É necessário mapear explicitamente cada usuário legado ao UUID Supabase verificado, mantendo seu user_id operacional em commercial_accounts. Não realizar associação automática por e-mail. Colisão de e-mail retorna identity_migration_required; contas legadas sem vínculo são negadas quando o gate é ligado. A comprovação da migração das contas legítimas e de rollback precede qualquer habilitação comercial.

Depois da validação e autorização, configurar no portal MCP_SERVICE_ORIGIN e MCP_SHARED_SECRET, na VPS REMOTE_SERVICE_COMMERCIAL_PORTAL_URL e REMOTE_SERVICE_COMMERCIAL_SECRET correspondentes. Configurar REMOTE_SERVICE_AVAILABLE=true somente após verificação do fluxo real. Nenhum desses segredos deve ir ao Git ou ao navegador.

## Antes de abrir vendas

Configurar DELIVERY_DEADLINE, SUPPORT_EMAIL, LEGAL_OPERATOR_NAME, REFUND_POLICY, ADMIN_USER_IDS e SMTP/callback Supabase; validar acesso administrador e estornos. Confirmar a nova migration, filtro PostgREST hospedado, ticket/identidade da VPS, primeiro comando e renovação, incluindo usuários legados. SALES_ENABLED=true, cobrança real de teste, deploy/ativação comercial da VPS e DNS exigem autorização explícita separada.

Não há renovação automática: cada cobrança avulsa compra um crédito de exatamente 2.592.000 segundos iniciado pelo primeiro comando remoto válido. Login, OAuth, pareamento, inspeção e pagamento antes da disponibilidade não iniciam o período.

## Evidência executada nesta revisão

- Portal: `npm test` com RDC_E2E_RUST_TEST_BIN apontando o teste Rust compilado da própria branch: **8 arquivos, 80 testes aprovados, zero skips**, 89,98 s. Typecheck aprovado; ESLint zero erros e três advertências preexistentes. Build Next.js/webpack aprovado. `npm audit --omit=dev`: zero vulnerabilidades informadas.
- E2E integrado: sete cenários aprovados, incluindo serviço Rust real, runner device-agent real, HTTP local, IPC autenticado com executor echo de teste, portal/Store/Billing reais e migrations no PGlite. A identidade passa pelo ticket real do portal/Rust. O grant OAuth e dispositivo operacional são fixtures no libSQL descartável; não se declara um pareamento/OAuth completo real nessa prova.
- No cenário conjunto, o runner permaneceu ativo antes da ativação; comando remoto retornou o resultado via relay/IPC, e SQL confirmou um direito ativado por exatamente 2.592.000 segundos e uma operação registrada.
- Testes RED observados antes das correções: autorização financeira sem reconciliação; função sem lista de pedidos verificados; poll ocioso READY_TO_ACTIVATE retornando 403. Regressões correspondentes passaram depois das alterações.

Para repetir a prova conjunta, primeiro compilar `cargo test -p device-agent --test commercial_portal --no-run`; selecionar o executável `target/debug/deps/commercial_portal-*.exe` recém-compilado e definir seu caminho absoluto em RDC_E2E_RUST_TEST_BIN, então executar `npm test` em apps/portal. Sem essa variável, o cenário conjunto fica explicitamente skipped; o teste Rust externo é ignored na suíte padrão e só roda com o harness HTTP/SQL ativo.
