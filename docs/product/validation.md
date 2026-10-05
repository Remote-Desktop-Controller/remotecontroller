# Validação integrada — versão 0.3.0

2026-10-05, Windows x64. Evidências locais preservadas no worktree; não são prova de hospedagem pública.

- Build e check de todo o workspace passaram.
- Suíte completa: 116 testes aprovados, quatro fixtures de processo ignoradas que são invocadas pelos testes de árvore/crash. Inclui oito fluxos HTTP, cinco regressões de concorrência/outbox, nove sessões, três testes de terminal e ciclo de instalação real do core.
- IPC v2: listener falso, autenticação incorreta, reflexão de prova, limite de frame e segredo privado validados. Componentes são distribuídos juntos.
- R1/R2 corrigidos no agente: slots de controle separados; resultados expirados/rejeitados arquivados com motivo/status e resultado inteiro, sem bloquear heartbeat/poll; retries transitórios preservados e justos. Quotas não removem evidência.
- R3 corrigido: context.recent/event herdam o filtro de sessão; regressão de duas sessões passou.
- R4 delimitado em security.md: Unix exige escritores cooperativos; detecção após efeito não promete rollback de inode substituído. Windows opera pelo handle verificado.
- Clippy estrito passou após ajustes automáticos de ifs e inicializadores, mais indireção do snapshot (mesmo JSON persistido).
- Teste adicional de crash do daemon pertencente ao agente passou: recupera workspace e segredo sem rotação. O agente supervisiona esse filho durante o relay; não substitui daemon de outro proprietário.
- Pacote/instalação Windows descartável passou: 12 arquivos verificados, PowerShell 5.1, reinstalação preservando estado, autostart com perfil contendo espaços e remoção do registro, recusa de checksum adulterado. Binários de debug; release ainda em validação.

Pendentes: build release, benchmark/release storage, execução dos binários release instalados, CI Linux/macOS/Windows e container, artefato final e validação visual autenticada. T08 não absorve esses gates.

## Limite da automação visual local

A página de login foi renderizada e os campos/botão têm validade, type submit, form action/method corretos. Os comandos de clique dos dois mecanismos documentados do navegador integrado não produziram navegação nem erro; o browser externo não está conectado. A submissão visual autenticada não foi comprovada por esse harness. O teste HTTP de formulário real, cookies, CSRF, register/login/logout passou na suíte; isso valida o servidor e não é declarado como clique real do navegador. A prova na interface real permanece explícita, sem enfraquecer CSRF ou inserir cookie artificial.

Container CI passou em 37305576185: compose válido, imagem não-root/read-only, cadastro de conta, reinício e login preservado. Linux completou a suíte e iniciou benchmarks. A captura móvel login-mobile.png tem largura de conteúdo/viewport 390/390. Não é captura autenticada e não prova acesso pela conta ChatGPT.

## Achado R5 — Origin em navegação de formulário (2026-10-05)

O teste Chromium isolado reproduziu POST /register com Origin:null e 403. A política no-referrer do serviço causa esse cabeçalho em navegações POST conforme [Fetch Standard](https://fetch.spec.whatwg.org/#append-a-request-origin-header). Os testes HTTP anteriores sintetizavam Origin correto e não capturavam o comportamento nativo.

Correção em execução: strict-origin no serviço e Caddy, preservando origem, sem enviar path/query no Referer. Origin estrangeiro/null continua recusado e CSRF permanece obrigatório. Novo gate tools/test-service-ui.mjs comprova cadastro/login, aprovação/revogação e logout por cliques em navegador com perfil/conta descartáveis. O CI recebeu esse gate. Enquanto a execução final não passa, T02 volta a validating; o score ponderado é 80, e nenhuma ação do proprietário absorve esta falha.

R5 validado após correção: Chromium real enviou Origin da própria página e aprovou cadastro, erro/login correto, pareamento com polling do agente simulado, revogação, logout e layouts 390/1280 sem overflow. Screenshots de dashboard são de conta/dispositivo descartáveis e não representam conta pública/PC de produção. T02 retornou a verified. O primeiro CI completo (37305576185) passou Windows/Linux/macOS/container; a mudança de cabeçalho e o novo gate exigem a rodada final antes de fechar T07.

## Fechamento de distribuição

O template do plugin ganhou ícone vetorial quadrado 512x512, campos logo/composerIcon e casos de revisão dentro de extensions.com.openai.review.test_cases. Build com URLs example.test descartáveis passou no schema oficial Agent Plugins 1.0.0; assets referenciados existem e os cinco casos positivos/cinco negativos têm prompt, descrição, ferramentas e resultado esperado. Nome/políticas/URLs reais e vídeo de demonstração da conta real permanecem T08.

A rodada 37309996209 passou Linux/macOS e container com navegador real. Windows falhou no observador do teste de uninstall: leu uninstall-status.json durante o lock exclusivo do worker e recebeu ERROR_SHARING_VIOLATION (32). O observador agora repete somente esse erro dentro do mesmo deadline de 15s, conserva rejeição de success=false e falha para demais erros. O teste focado usando release instalado passou. A próxima execução será somente Windows, pois runtime/ferramentas não mudaram desde os gates Linux/macOS/container desta rodada; a diferença é teste Windows e metadados de distribuição.

R6: a falha do observador Windows foi corrigida e o teste de release instalado passou em lifecycle-observer-fix.log. Não foi alterado o worker nem relaxado o critério de sucesso/deadline. A execução final Windows está em 37313011274. O serviço/ferramentas são idênticos à fonte validada Linux/macOS/container em 37309996209 (git diff de src/Cargo/migrations/deployment vazio).

O construtor do plugin aceita DemoRecordingUrl para preencher a gravação real sem edição manual de JSON. O smoke com URL example.test passou e não representa publicação.

## Gates finais aprovados

2026-10-05: Windows 37313011274 terminou success, incluindo fmt/check/Clippy, suíte, libSQL ownership, oito benchmarks, storage/release, executáveis instalados via MCP, agente remoto release, instalação/autostart do produto e pacote. Linux/macOS e container/browser terminaram success em 37309996209. Runtime src/Cargo/migrations/deployment iguais entre essas revisões; somente o observador Windows e metadados de distribuição foram ajustados.

Os três ZIPs de CI foram baixados para dist/ci e verificados por SHA256 externo e 12 hashes internos. artifacts.json registra paths/arquiteturas/hashes. O ZIP Windows local com manual atualizado está em dist/delivery; SHA256 214873da2a53d894729f247072411bf5cee806d3a01bb5d2da71da22e9e78c12. Esse build GNU foi instalado/testado neste PC; CI Windows usa o toolchain nativo do runner. Os componentes têm versão 0.3.0 e IPC2.

Marco técnico: 99/100. T08 é a única tarefa restante e inclui domínio/host, callback real, identidade/políticas, teste ChatGPT web/móvel, gravação/revisão/publicação e assinatura quando exigida. Produto público validado: false. Esses passos não foram executados por falta de titularidade/conta/credenciais, conforme o escopo manual aprovado.

## Correção de compatibilidade OAuth após teste público (2026-10-05)

RFC 9207 implementada em oauth.rs, com flag e issuer exato no sucesso/erro redirecionado. As regressões falharam antes do patch e passaram depois. Allowlist permanece explícita, PKCE S256/resource/redirect exato preservados e CIMD continua desabilitado. Ver oauth-rfc9207.md para evidências e passos de rebuild/configuração do servidor; deployment e teste real ChatGPT aguardam o proprietário. Os gates locais completos passaram nesta revisão.
