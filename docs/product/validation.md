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
