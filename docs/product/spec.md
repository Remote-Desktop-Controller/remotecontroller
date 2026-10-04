# Produto remoto — desenho aprovado para execução

Data: 2026-10-04. Base: ba5ee3a. Autorização: o usuário aprovou a conclusão autônoma do desenho apresentado, decisões moderadas e somente as ações humanas inevitáveis ao final.

## Objetivo e limites

Instalar o agente, entrar em uma conta, parear um computador e usar arquivos e processos pelo ChatGPT web/celular. Preservar o runtime Rust existente, permissões, recuperação e auditoria. Memória operacional deve permitir retomada sem repetir efeitos colaterais. Não prometer treinamento do modelo, contexto ilimitado, aprovação de catálogo ou ausência de bugs.

## Arquitetura

Adicionar `remote-service` em Rust (Axum + libSQL) e `device-agent` em Rust (reqwest + IPC existente). O serviço publica MCP Streamable HTTP com respostas JSON, discovery OAuth e autorização PKCE S256. O agente faz conexão de saída por HTTPS, com polling limitado e reconexão; o PC não abre porta pública. HTTP é permitido somente com opção explícita em loopback para testes.

Contas locais do serviço usam password hashing Argon2, cookie HttpOnly/SameSite, CSRF nas mutações e limitação de tentativas. Tokens opacos de alta entropia ficam hashados no banco. OAuth inclui consentimento explícito, audience/resource, escopos, expiração, refresh rotation e revogação. Cadastro OAuth público deve validar redirects por política explícita; não permitir SSRF/CIMD arbitrário. O fluxo deve funcionar com registro dinâmico ou clientes pré-configurados, conforme documentação oficial.

Dispositivos são vinculados a contas por código curto de uso único, expiração e polling autenticado por segredo independente. Cada chamada resolve o proprietário do token e verifica a propriedade do dispositivo. Transporte envia o envelope IPC existente, preservando operation_id nos retries e session_id quando disponível. Fila e resultados são limitados; operações mutantes não são reenviadas com IDs novos. Revogação interrompe novas chamadas. Um daemon continua controlando um workspace; o produto permite múltiplos dispositivos/agentes.

## Contrato HTTP entre agente e serviço

Todos os corpos JSON; UUIDs em strings. Erros HTTP com `error` e mensagem sem segredos. `Authorization: Bearer` nas rotas do agente.

- POST `/api/pairings`: `{name, workspace_id, tools: ToolDescriptor[]}` -> `{pairing_id, pairing_code, verification_uri, poll_token, expires_at}`. Sem conta; limitado por origem.
- POST `/api/pairings/{id}/poll`: `{poll_token}` -> `{status: pending|approved|expired, device_id?, device_token?}`. O segredo do agente é entregue somente a quem iniciou o pareamento.
- POST `/api/pairings/approve`: sessão web + CSRF + `{code}`. Aprovação explícita pela conta.
- POST `/api/agent/heartbeat`: `{workspace_id, tools}` -> estado; persiste catálogo e presença.
- GET `/api/agent/poll`: resposta `{commands: [{request_id, envelope: RequestEnvelope}]}`, timeout de até 25 s; fila limitada.
- POST `/api/agent/result`: `{request_id, envelope: ResponseEnvelope}` -> confirmação. Conferir que a requisição pertence ao dispositivo e a resposta ao operation_id esperado.
- UI autenticada `/`: computadores, pareamento, revogação, instruções de conexão; login/cadastro/logout. Sem comandos arbitrários no navegador.
- `/mcp`: OAuth protegido; tools `device.list`, `device.select` e ferramentas do catálogo do dispositivo escolhido. `device_id` opcional escolhe dispositivo autorizado; `session_id` opcional propaga ao IPC; parâmetros remotos são retirados antes de chamar tools locais.

## Continuidade e ferramentas locais

Migração aditiva de sessões e vínculo com operações, preservando bancos v0.2. `session.create/list/resume/close` com workspace isolation; gateway aceita `_meta.runtimeSessionId`. Leitura/busca de spool por offsets sem remover quotas, com próximo offset, estado de captura, bytes armazenados e gerados. Históricos antigos devem ser consultáveis com busca paginada. Grafo mantém relações temporais separadas de dependências; não reintroduzir CTE problemática sem prova.

Completar operações de arquivos com confinamento de paths e journal/rollback: diretórios, rename/move e remoção explícita. Terminal interativo deve usar PTY/ConPTY e as mesmas aprovações do processo; não liberar shell genericamente sem autorização local clara.

## Validação e conclusão

Testes reais de HTTP + IPC + daemon, isolamento entre duas contas, revogação, PKCE errado/replay, refresh replay, pareamento expirado/replay, reconexão e repetição sem duplicação. Validar UI no navegador, instalação, reinício e execução remota. CI Windows/Linux/macOS e build de distribuição. Evidência local não substitui deploy real nem invocação ChatGPT.

Separar `prontidão técnica` de `produto público validado`. A meta 99% só pode ser atribuída a entregas verificadas; registrar pendências automáticas, humanas e de plataforma sem esconder trabalho de engenharia no último 1%.

## Dependências humanas inevitáveis

Conta de hospedagem e domínio/HTTPS se indisponíveis; titularidade/identidade/termos/políticas; certificado de assinatura quando necessário; aprovação/publicação no catálogo; teste com conta real ChatGPT. Preparar scripts, pacotes e instruções antes de pedir qualquer ação.
