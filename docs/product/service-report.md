# Serviço remoto — implementação e evidência

Escopo: `apps/remote-service`. Serviço Rust/Axum 0.8 com libSQL `=0.9.30`, usando o patch local já existente no workspace. Não executa ferramentas, shell ou operações de arquivos; transporta `runtime-protocol` para o agente/daemon que mantém a política local.

## Implementado

- Contas com Argon2id, senhas de 12–256 caracteres, comparação de senha fora das threads de I/O e quatro trabalhos de senha simultâneos. Sessões aleatórias de 256 bits, somente hashes no banco, validade de 24 horas, cookie HttpOnly/SameSite=Lax/Secure em HTTPS. CSRF em mutações autenticadas e em formulários de login/cadastro. Limites por origem TCP e conta, mapa de rate limit com no máximo 4.096 entradas e expiração; cabeçalhos forwarded não são usados para decidir identidade.
- UI responsiva em português: login, cadastro, computadores, presença, aprovação de pareamento, revogação de dispositivo, conexões autorizadas, revogação de conexão, logout e consentimento explícito OAuth. Continuação de autorização é preservada ao alternar entre login e cadastro.
- Discovery OAuth AS e protected-resource (incluindo caminho `/mcp`), challenge `WWW-Authenticate`, registro dinâmico de clientes públicos, redirects validados por allowlist **exata**, PKCE S256 obrigatório, audience/resource estrito em autorização/troca/refresh/uso, escopos `devices:read tools:read tools:execute`, codes de cinco minutos e uso único, access de uma hora, refresh de 30 dias com rotação e detecção de replay que revoga a família inteira. Revogação OAuth por token ou UI. CIMD não é anunciado nem buscado: nenhuma URL arbitrária de cliente é acessada pelo serviço.
- Pareamento inicia sem sessão, código humano de 40 bits com validade de dez minutos, aprovação autenticada/CSRF, segredo independente de polling de 256 bits, token privado do dispositivo hashado. Aprovação e entrega são transacionais e de uso único. Workspace de um dispositivo é imutável. Catálogo/presença são atualizados por heartbeat autenticado.
- Relay durável em libSQL/WAL, transações immediate, isolamento por conta/dispositivo/família OAuth, polling até 25 segundos, lease de 30 segundos e repetição do **mesmo envelope e operation_id**. Resultado verifica todos os campos de Metadata; replay idêntico de resultado é idempotente e resultado divergente é rejeitado. Revogar dispositivo exclui comandos ainda sem resultado; revogar conexão impede novas entregas da família. Resultados entregues continuam consultáveis pelo mesmo operation_id.
- MCP Streamable HTTP em modo stateless JSON, initialize/ping/tools/list/tools/call, 202 para notificações e 405 para GET/DELETE autenticados. Protocolos 2025-03-26, 2025-06-18 e 2025-11-25. `device.list`/`device.select`, catálogo local e forwarding com aprovação local preservada. Um único computador é escolhido automaticamente para que o host descubra suas ferramentas já na primeira listagem; para múltiplos sem seleção, o catálogo combina nomes e exige `device_id`. `device_id` remoto é retirado dos argumentos locais. `session_id` é retirado e propagado quando não pertence ao schema da ferramenta; nas ferramentas que usam `session_id` como argumento local, ele é preservado e o contexto usa `_meta.runtimeSessionId`. O argumento visível `runtime_operation_id` (ou `_meta.runtimeOperationId`) permite retomada explícita e deduplica esse comando; sem ele, cada chamada ganha identidade nova. Os dois campos conflitantes são rejeitados. `runtime_operation_id` é retirado dos argumentos locais; `operation_id` usado pelas ferramentas locais de consulta/cancelamento é preservado. O id JSON-RPC serve somente para correlação e pode ser reutilizado por chats distintos sem suprimir comandos intencionais.
- Timeout da resposta MCP devolve estado `pending` com request/operation IDs, sem declarar falha ou criar outra operação. Após o prazo de entrega, o retorno explicita execução incerta e instrui consultar a operação antes de usar identidade nova.

## Limites operacionais deliberados

Corpo HTTP 1 MiB; requisição relay 512 KiB; resultado relay 768 KiB; catálogo 128 ferramentas e 256 KiB por dispositivo; nome até 120 caracteres; até 20 dispositivos ativos e 100 registros de dispositivos por conta. Até 1.024 pareamentos ativos globais; 10.000 clientes DCR; 4.096 registros de grant por conta dentro da validade de refresh. Fila padrão: 64 comandos não concluídos por dispositivo; no máximo 256 comandos/resultados por dispositivo retidos por 24 horas. O limite total evita crescimento ilimitado; uma conta que atinja a retenção recebe HTTP 429 até a expiração. Poll retorna até oito comandos por entrega. Há 128 requisições simultâneas de espera entre polling e chamadas remotas.

Os limites de taxa ficam em memória por instância e reiniciam com o processo. Usar também limites no proxy frontal. O banco local é serializado por conexão/mutex; transações são curtas e nenhum long-poll retém a conexão durante a espera. Para operação pública com alta concorrência, medir a carga real e dimensionar o proxy/instância antes de escalar.

O token de dispositivo é entregue **uma vez** e nunca fica em texto puro no banco do serviço. Se a resposta de entrega se perder antes de o agente salvar a credencial, o usuário precisa iniciar outro pareamento. O polling posterior informa aprovado/dispositivo, sem recuperar o token. Isto evita armazenamento de segredo recuperável; é uma limitação explícita de recuperação desse estágio. Um comando já entregue/em execução no daemon não pode ser desexecutado pela revogação remota.

## Compilação e execução

Adicionar `apps/remote-service` aos membros do workspace (integração do root). Comandos de validação centralizada:

```powershell
cargo check -p remote-service --all-targets
cargo test -p remote-service -- --nocapture
cargo build --release -p remote-service
```

Produção, atrás de proxy HTTPS confiável, mantendo a porta interna privada:

```text
REMOTE_SERVICE_BIND=0.0.0.0:8787
REMOTE_SERVICE_PUBLIC_URL=https://seu-dominio
REMOTE_SERVICE_DATABASE=/data/service.db
REMOTE_SERVICE_REDIRECT_URIS=https://callback-exato-autorizado
```

`REMOTE_SERVICE_REDIRECT_URIS` aceita lista separada por vírgulas; obtenha o callback real do cliente/plataforma em vez de assumir caminhos. Sem essa lista, DCR de callback HTTPS é rejeitado. O serviço não busca URLs de redirect, logo não cria SSRF por esse mecanismo.

Teste local somente com opção explícita e loopback:

```powershell
remote-service --public-url http://127.0.0.1:8787 --allow-http-loopback --database .data/service.db --redirect-uris https://client.example/callback
```

Flags correspondentes: `--bind`, `--public-url`, `--database`, `--allow-http-loopback`, `--redirect-uris`. Não habilitar a exceção HTTP em produção. `/healthz` retorna somente status; `/api/session` e `/api/me` retornam a sessão autenticada e CSRF; `/api/devices` retorna dispositivos da conta.

API de conta: POST `/api/auth/register` e `/api/auth/login` recebem JSON `{email,password}`, retornam `{user_id,csrf_token}` e cookie `rdc_session` (201/200). GET `/api/session` retorna `{user_id,email,csrf_token}`. Mutações JSON autenticadas exigem `x-csrf-token`. OAuth token/revoke/consent usam formulários `application/x-www-form-urlencoded`, conforme OAuth; pairings e rotas do agente usam JSON conforme o spec.

## Evidência de testes

Testes unitários: vetor RFC7636 de PKCE, verifier incorreto/curto, redirects exatos, rejeição de fragment/credentials/host parecido e exceção loopback explícita.

Testes HTTP em `tests/flows.rs`: isolamento entre duas contas, CSRF/revogação de dispositivo invalidando agente; segredo de polling errado, aprovação replay e entrega única; Origin hostil e DCR não autorizado; code PKCE/resource errado, code replay, refresh rotation/replay revogando novo access; pareamento expirado; heartbeat/catálogo, fila durável/reabertura do serviço, session metadata e argumento local de session.resume, isolamento de MCP/result entre contas/dispositivos, identidade divergente, result replay, retry estável, conflito de mesma operação e quotas; revogação OAuth impede entregas pendentes. Chamadas intencionais sem operation identity e com id RPC igual devem gerar operações diferentes; retries com o argumento visível devem reutilizar a operação; meta/arg conflitantes são rejeitados.

Status da execução centralizada inicial: os dois unitários e cinco dos seis testes HTTP passaram. O teste relay revelou erro de armazenamento; tracing identificou `Null value` e a investigação no código oficial do libSQL confirmou que Row local referencia a posição corrente do Statement. A autoseleção guardava Row e avançava o cursor antes de copiar o ID. Foi corrigida a ordem de leitura, mantendo os critérios de teste; a rodada focused passou (`service-relay-green.log`, um teste em 5,81 s). Depois foram incorporados os ajustes de identidade explícita e retry por argumento visível; a rodada final e a integração HTTP real agente/daemon ficam registradas pelo root nos arquivos de evidência do produto. Não houve execução de Cargo concorrente iniciada por esta frente.

## Publicação e riscos restantes

Os endpoints e a UI são implementação real, mas este relatório não prova hospedagem HTTPS pública, distribuição assinada, onboarding no catálogo nem invocação pela conta real do ChatGPT. Esses itens precisam de evidência separada. Também não implementa envio de e-mail/redefinição de senha; as contas locais atuais exigem que o usuário guarde a senha. Proteger/backup do banco e titularidade do domínio permanecem responsabilidades da implantação.

Referências oficiais consultadas: [MCP Authorization 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization) e [MCP Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). A implantação escolhe DCR com política de redirect explícita, PKCE e resource binding; não afirma suporte CIMD nem aprovação em catálogo.
