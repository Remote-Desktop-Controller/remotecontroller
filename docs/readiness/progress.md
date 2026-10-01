# SDD ledger — plan: docs/readiness/plan.md

**Candidata v0.2 concluída e publicada em 2026-10-01.** Código integrado à main
pela [PR #1](https://github.com/Remote-Desktop-Controller/remotecontroller/pull/1).
[Release v0.2.0](https://github.com/Remote-Desktop-Controller/remotecontroller/releases/tag/v0.2.0)
com Windows x64 (GNU), Linux x64 e macOS arm64, manifests e SHA-256 verificados.
[CI final](https://github.com/Remote-Desktop-Controller/remotecontroller/actions/runs/36880261171)
passou nos três sistemas no commit `aaf579a`; a árvore integrada em `81b9ddc`
é idêntica à candidata testada.

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
## Registro inicial v0.2 — 2026-09-30

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

## Fechamento retomado — 2026-10-01

Correções finais confirmadas por regressões:

- ACL legada Windows: `SetSecurityInfo` acrescentava ACEs herdadas. O descriptor
  original agora é restaurado pelo handle com `NtSetSecurityObject`; RED e GREEN
  locais estão em [dacl-red](../evidence/dacl-red-v0.2.log) e
  [dacl-green](../evidence/dacl-green-v0.2.log).
- libSQL 0.9.30 fechava duas vezes a conexão nativa. Patch local de uma linha
  torna o fechamento idempotente, preservando o resto do código/dependências.
  [RED](../evidence/libsql-close-red-v0.2.log),
  [GREEN](../evidence/libsql-close-green-v0.2.log) e
  [proveniência](../../vendor/libsql/PATCH.md).
- Configuração de política agora deve ficar fora do workspace, por caminho
  lexical e resolvido, incluindo links e arquivos ainda ausentes. Temporários,
  backups e configuração registrada são privados antes de gravar os bytes.
- Backup/retention Unix sincroniza as pastas copiadas em pós-ordem e o parent
  do banco/audit antes da primeira remoção. O teste Linux observa os syscalls
  reais com strace, além de conferir os conteúdos preservados.

As três regressões Unix falharam no código anterior pelos motivos esperados:
[runner RED](https://github.com/Remote-Desktop-Controller/remotecontroller/actions/runs/36872436503),
[log](../evidence/unix-regressions-red-v0.2.log). Após a correção, o mesmo runner
passou os testes de durabilidade, duas permissões e sete casos de configuração:
[runner GREEN](https://github.com/Remote-Desktop-Controller/remotecontroller/actions/runs/36873117889),
[log](../evidence/unix-regressions-green-v0.2.log).

Suíte Windows anterior ao último follow-up: 63 testes, zero falhas; crash de
lote/reconexão e árvores de processo passaram. Stress terminou em 115,34 s.
Os oito benchmarks finais terminaram com código 0, inclusive causal_query;
storage em release também passou (dois testes). Logs:
[tests-final](../evidence/tests-final-v0.2.log),
[criterion-final](../evidence/criterion-final-v0.2.log),
[storage-release](../evidence/storage-release-final-v0.2.log).

Revisão independente da candidata completa identificou as três lacunas acima;
o follow-up aprovou as correções sem pendência Critical/Important. O CI agora
executa ownership do libSQL, benchmarks reais, storage otimizado e MCP/lifecycle
com os executáveis release.

Gates locais após follow-up concluídos: fmt/check/Clippy all-targets/all-features,
build, **66 testes Windows com zero falhas** e quatro fixtures de processo
invocados pelos próprios testes. Último stress: 103,79 s, com todos os conteúdos
verificados. [Testes revisados](../evidence/tests-reviewed-v0.2.log),
[check](../evidence/check-reviewed-v0.2.log),
[Clippy](../evidence/clippy-reviewed-v0.2.log),
[build](../evidence/build-reviewed-v0.2.log).

Build release final passou. Os executáveis release passaram configuração (3),
crash/MCP/processos (3) e instalação/update/uninstall (1):
[release-fullstack](../evidence/release-fullstack-final-v0.2.log).
Imports do GNU local contêm apenas DLLs Windows/UCRT:
[inventário](../evidence/release-imports-final-v0.2.log).
ZIP Windows v0.2.0 e manifest foram verificados por SHA-256:
[pacote](../evidence/package-final-v0.2.json).

O CI tri-OS final passou e os pacotes da release publicada foram verificados.

Follow-up macOS: o runner encontrou configuração aceita quando aliases distintos
de workspace/parent (`/var` e `/private/var`) antecediam um link para fora do root.
A checagem agora verifica cada ancestral existente por caminho resolvido, além
das verificações lexicais e do destino final. A regressão original e um caso
Unix portátil com aliases distintos cobrem esse caminho.
[RED macOS](../evidence/macos-alias-red-v0.2.log). Fmt/Clippy, configuração Windows,
rebuild release e os sete casos MCP/lifecycle/config passaram após esse ajuste:
[config-debug](../evidence/alias-config-debug-v0.2.log),
[release-MCP](../evidence/alias-release-mcp-v0.2.log). O novo CI tri-OS passou,
incluindo os oito casos de configuração Unix e os binários release.

Evidência remota final:
[resumo CI](../evidence/ci-final-v0.2.json),
[Windows](../evidence/windows-final-ci-v0.2.log),
[Linux](../evidence/linux-final-ci-v0.2.log),
[macOS](../evidence/macos-final-ci-v0.2.log),
[ZIPs/manifests verificados](../evidence/release-packages-v0.2.json).
O Windows publicado usa o GNU validado localmente; o pacote MSVC também passou
os gates do CI. Ambos compartilham o mesmo código candidato. Assinatura pública
de editor/notarização permanece requisito externo, como previsto na spec.
