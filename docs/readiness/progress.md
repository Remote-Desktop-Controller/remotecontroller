# SDD ledger — plan: docs/readiness/plan.md

Base: 94a2a98, CI tri-OS verde. Memória do projeto consultada: sem entradas relevantes.

Pre-flight:

| Tarefas | Interface compartilhada | Ruling |
|---|---|---|
| A/D | FileSnapshot + serialization | A preserva compatibilidade de snapshots antigos; root integra migration |
| B/D | LocalStore/state dir/config | root expõe state_dir(); B mantém constructor existente quando possível |
| C/D | ContextStoragePort/tools/config | C adiciona API compatível, root expõe tool inputs e maintenance commands |
| E/D | IPC lifecycle/config | bootstrap pertence ao daemon; gateway permanece adapter |
| A/B/C | Cargo checks | root serializa os gates; agentes não executam builds amplos simultâneos |
| F/E | signing | credencial externa necessária; sem assinatura fictícia |

Ruling: preservar seis libs e dois binaries nesta evolução. Revisar interfaces
sem uso, integrando contratos reais quando úteis; não reduzir contagem por estética.
## Estado v0.2 em finalização

| Bloco | Implementação e evidência local | Gate restante |
|---|---|---|
| A — arquivos | Snapshot compatível com legado, metadados nativos, hashes e rollback protegido; regressões Windows passaram | CI Unix/macOS |
| B — processos | Aprovação/pin, spool sincronizado e quotas, Job antes de resume, guardian Unix; cancelamento de árvore e crash real passaram no Windows | CI Unix/macOS |
| C — contexto/manutenção | Task/query/rollups e budget UTF-8; backup privado quick_check, retenção com audit, ativos/prepared/tmp de crash passaram | CI Unix/macOS |
| D — integração | Contratos PermissionPort/Context/IpcTransport usados, config bounded, shutdown e heartbeat com fingerprint; gates locais passaram | CI |
| E — instalação | Install/connect/doctor/stop/update/uninstall/register-host; MCP real, update, self-uninstall e gateway em uso passaram; Codex CLI reconheceu configuração preservada | CI Unix/macOS |
| F — distribuição | Workspace versionado 0.2.0; script ZIP/manifest/checksum e workflow tri-OS | Release build, CI e publicação |

Suite `cargo test --workspace --locked`: **60 passaram, zero falhas**, quatro
fixtures ignorados pelo runner principal e invocados como processos filhos pelos
testes. Stress passou em 127,46 s; todos os 10.000 conteúdos foram conferidos.
Log: [tests-v0.2](../evidence/tests-v0.2.log).

Clippy all-targets/all-features `-D warnings` e fmt passaram no estado revisado:
[clippy-v0.2](../evidence/clippy-v0.2.log). Check all-targets passou.
Reteste da revisão: **39 passaram, zero falhas**, incluindo novo teste de update
rejeitado sem alterar ativação, self-uninstall e recusa com gateway real mapeado:
[review-regressions](../evidence/review-regressions-v0.2.log).
Native MCP e host real são evidências distintas: handshake/tools/list pelos
binários reais no teste lifecycle; configuração aceita pelo `codex mcp get`
real em perfil isolado: [codex-host](../evidence/codex-host-v0.2.json).
Não houve chamada de modelo cloud nem alteração da configuração pessoal do host.

Revisão independente confirmou: `.await` ausente no caminho Unix do guardian,
cap de backup incompatível com o máximo de registros, `metadata.tmp` de crash
impedindo retenção e self-uninstall de executável mapeado no Windows. Correções
integradas e retestes passaram. A recusa de executável mapeado usa
FileDispositionInfoEx/FORCE_IMAGE_SECTION_CHECK; markers de deletion são cancelados
antes de fechar handles se algum membro estiver em uso. O alegado conflito
de fingerprint foi retirado após confirmar `#[serde(skip)]` em guardian_path.

O teste histórico que restaurava edição externa direta foi atualizado para uma
mutação registrada pelo runtime. Novo teste comprova que bytes externos são
preservados; snapshots legados alterados sem guard continuam recusados.

Assinatura pública permanece dependente de certificado externo; nenhum certificado
de code signing está disponível nesta máquina. O pacote utiliza checksums e não
declara sandbox de OS nem isolamento contra malware do mesmo usuário.
