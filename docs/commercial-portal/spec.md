# Portal comercial — especificação de execução

Autoridade: pedido anexado pelo responsável em 08/10/2026. Branch `feat/commercial-portal`, portal em `apps/portal`. Blog e DNS de `toemdia.blog.br` não serão alterados. Publicação inicial independente em `vercel.app`.

## Decisões

- Next.js 16 + TypeScript + Tailwind CSS 4, Supabase Auth com e-mail confirmado e PostgreSQL com RLS. Sem credenciais no código. Venda avulsa PIX via API Asaas, preço em centavos configurado no servidor e fotografado no pedido. Sem assinatura recorrente ou renovação automática.
- A documentação entregue deve impedir pagamentos reais enquanto prazo de entrega, operador, suporte e política de reembolso não forem configurados. Sandbox pode testar o fluxo sem esses dados comerciais reais. Serviço MCP indisponível por padrão.
- Direito adquirido não expira durante pré-venda. Login no portal, OAuth, cadastro de dispositivo, descoberta MCP e reconexão não iniciam prazo. Apenas uma operação remota válida, após verificação de scope, dispositivo, ferramenta e admissão local, solicita autorização comercial `execute`.
- O banco comercial não substitui libSQL operacional. O serviço Rust vincula identidade Supabase via ticket curto de uso único e nonce de login armazenado em libSQL/cookie. A associação não se baseia em e-mail digitado nem em IDs enviados pelo navegador.
- A cobrança local do trabalho anterior não será copiada para esta branch: ela era mensal e iniciava no login, incompatível com o novo pedido. Partir do `codex/remote-product` HEAD validado, preservando alterações anteriores naquele checkout.
- Se há direitos pré-pagos adicionais, só se ativa o próximo após terminar o atual, no próximo uso remoto autorizado. Cada direito mantém exatamente 2.592.000 segundos. Não reiniciar/estender um direito por login, retry, dispositivo ou refresh OAuth.
- Cancelar pedido não pago encerra a cobrança; pedido pago exige reembolso confirmado pelo Asaas para retirar direito. Solicitação de reembolso não é comprovante de dinheiro devolvido. Estorno efetivo/parcial revoga o direito correspondente.

## Contratos do portal HTTP

User-facing POST exige sessão Supabase validada pelo servidor e Origin igual ao portal. Webhook e APIs servidor-servidor usam segredos próprios; não usam Origin como autenticação.

| Rota | Contrato |
| --- | --- |
| GET `/api/config` | `{ priceCents:number|null, salesEnabled:boolean, sandbox:boolean, serviceAvailable:boolean, deliveryDeadline:string|null, supportEmail:string|null, operatorName:string|null, refundPolicy:string, remoteOrigin:string|null, agentDownloadUrl:string|null }` sem segredos |
| POST `/api/auth/register` | `{email,password}` → `{message,emailVerificationRequired:true}` |
| POST `/api/auth/login` | `{email,password}` → `{ok:true}` ou erro; só e-mail confirmado |
| POST `/api/auth/logout` | `{}` → `{ok:true}` |
| GET `/auth/callback` | confirmação/PKCE verificados pelo Supabase; redirect local seguro |
| GET `/api/account` | `{user:{id,email}, subscription:{state,pendingDays,activatedAt,expiresAt},orders:[{id,amountCents,status,paymentUrl,createdAt}],serviceAvailable,salesEnabled}` |
| POST `/api/checkout` | `{name,cpfCnpj,termsAccepted:true}` → `{orderId,paymentUrl}`; cliente não escolhe valor/user |
| POST `/api/orders/{id}/sync` | concilia um pedido de propriedade comprovada → estado |
| POST `/api/orders/{id}/cancel` | cancela pedido pendente, sem invalidar pagamento recebido na corrida |
| POST `/api/orders/{id}/refund-request` | registra pedido de suporte, não executa estorno por navegador |
| POST `/api/asaas/webhook` | header `asaas-access-token`, consulta API e processa evento idempotente |
| GET `/api/admin` | allowlist de UUIDs Supabase exclusivamente no servidor; resumo/consulta |
| POST `/api/admin/refund` | admin autenticado + confirmação explícita; reserva e reconciliação do estorno |

Respostas de erro `{error:string,message:string}`, sem dados sensíveis. HTTP 401 sessão inválida, 403 IDOR/CSRF/admin, 409 operação ambígua, 429 limite, 503 configuração/provedor indisponível.

## Contratos portal ↔ Rust (imutáveis entre tarefas)

- Variáveis no portal: `MCP_SERVICE_ORIGIN` (origem HTTPS da VPS futura), `MCP_SHARED_SECRET` (>=32 caracteres, servidor somente), `REMOTE_SERVICE_AVAILABLE=false` por padrão.
- Variáveis Rust: `REMOTE_SERVICE_COMMERCIAL_PORTAL_URL` (origem HTTPS do portal), `REMOTE_SERVICE_COMMERCIAL_SECRET` (mesmo segredo). Ambas ausentes preservam comportamento legado. Ambas presentes habilitam modo comercial explicitamente. Valores parciais/URLs com credenciais/query/fragment/secret curto falham ao iniciar. HTTP somente em fixture loopback explicitamente autorizada.
- Rust `/login` gera nonce seguro, salva `next` local validado e prazo em libSQL, seta cookie HttpOnly SameSite=Lax e redireciona para `{portal}/connect?state={nonce}&redirect_uri={service}/auth/commercial/callback`.
- Portal `/connect` pede login se necessário e chama GET `/api/mcp/connect?state=...&redirect_uri=...`. Backend valida callback exatamente contra `MCP_SERVICE_ORIGIN`, autentica usuário Supabase com e-mail confirmado, grava hash de ticket aleatório 32 bytes, state, audience, TTL 60s. Resposta `{redirectUrl}` aponta apenas para callback conhecido com `ticket` e `state`.
- Rust `/auth/commercial/callback` exige cookie/nonce iguais e nonce local vigente, não consumido. POST `{portal}/api/mcp/exchange` com Bearer secret, `{ticket,state,redirect_uri}`. Portal consome ticket atomicamente, verifica state/audience/TTL e retorna `{subject:<uuid Supabase>,email,email_verified:true,audience:<MCP_SERVICE_ORIGIN>}`. Rust confirma tipos/audience, cria vínculo local estável, sessão e retoma `next`. Não associar conta local anterior apenas pelo e-mail: conflito exige migração administrativa explícita.
- POST `{portal}/api/mcp/authorize`, Bearer secret, body `{subject:<uuid>,operation_id:<uuid>,mode:"inspect"|"execute"}` → `{subject,operation_id,allowed:boolean,state:<enum>,expires_at:<epoch seconds|null>}`.
- `inspect` nunca ativa. `allowed=true` somente para ACTIVE vigente e serviço disponível. READY_TO_ACTIVATE é elegível para setup/metadata, sem autorização para entregar comando. `execute` ativa atomicamente o primeiro direito disponível quando serviço está disponível, ou reutiliza o vigente; em repetição não concede outro direito e não reinicia prazo.
- Rust verifica sempre identidade e operation_id da resposta. O user_id do grant resolve para commercial_subject persistido, nunca é escolhido pelo RPC. Acesso a URL MCP, token antigo, novo grant ou dispositivo não muda esse vínculo.
- Antes de enqueue, entrega ao agente e leitura de resultado, validar acesso atual; se portal falhar, negar operações comerciais (sem fallback livre). Chamadas externas ficam fora das transações/locks de banco. Resultado de comando já iniciado pode ser entregue ao gateway mesmo após vencimento, mas o gateway não libera nova operação sem direito.
- Não consumir um segundo crédito apenas para reconsultar operação idempotente expirada: a identidade da operação fica auditada no portal e retries não reativam direito. Ativação só após admissão local válida; operações reservadas não são entregues ao agente até autorização comercial.

## Validação

Testes de lógica/API e PostgreSQL (PGlite quando banco externo não disponível), incluindo RLS/IDOR, locks/idempotência, ciclo de estados e 30 dias. Fixtures HTTP isoladas para Asaas e portal comercial. Rust: testes existentes + testes de identidade/ticket e bypass/gates. Next: typecheck, build, lint quando aplicável, testes, screenshots mobile/desktop. Testes externos dependem de credenciais e devem ser distinguidos dos locais.
