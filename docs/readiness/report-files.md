# A — Integridade de arquivos

## Interfaces para integração

- `FileSnapshot::new(path, content)` mantém uma forma simples de construir fixtures/snapshots legados. Campos novos: `metadata: Option<FileMetadata>` e `restore_precondition: Option<FilePrecondition>`.
- `FilePrecondition::{Missing, Sha256([u8; 32])}` é domínio puro, sem serde. O root persiste a precondição pretendida antes da primeira mutação e conserva checkpoints quando há conflito.
- `WorkspaceFiles::snapshot_sync(path)` captura bytes/metadados, confere estabilidade e faz uma segunda leitura por handle relativo.
- `check_snapshot_sync(&snapshot)` compara bytes e metadados conhecidos antes da aplicação.
- `apply_snapshot_sync(&snapshot, Option<&[u8]>)` aplica a mudança preparada, verifica novamente antes do rename e retorna o snapshot de rollback com precondição do estado pretendido.
- `restore_sync(&snapshots)` faz preflight do lote inteiro: aceita arquivos já originais; restaura apenas estados que correspondem à precondição persistida; recusa conteúdo ou metadados alterados por writer externo. Falha parcial posterior mantém o journal para nova decisão/recovery.
- `SnapshotDto` e conversões `From` pertencem exclusivamente à infraestrutura. Metadata/guard ausentes no JSON antigo são decodificados como `None`. Timestamps têm representação signed-epoch, incluindo datas anteriores a 1970.

## Preservação concreta

Temporário, rename e limpeza usam um handle `cap_std::fs::Dir` do diretório pai. Não há std-write por path ambiente nem cópia de todo o workspace. Conteúdo é sincronizado depois da restauração de metadados; Unix sincroniza o diretório pai após rename/delete.

Propriedades capturadas/restauradas e verificadas no temporário antes da troca:

- todos: timestamps accessed/modified e readonly;
- Linux/macOS: mode (inclusive executable bits), uid/gid e extended attributes por fd; erros de fchown/xattr interrompem a operação;
- Linux: POSIX access ACL é preservada no xattr nativo `system.posix_acl_access`;
- macOS: extended ACL por `acl_get_fd`/`acl_to_text` e `acl_from_text`/`acl_set_fd`; ACL vazia é canonicalizada;
- Windows: criação, atributos básicos (readonly/hidden/system/archive/normal/temporary) e DACL self-relative/protection por APIs nativas de handle. Descritores persistidos passam por checagem de offsets/tamanhos antes da restauração.

A escritura normal também mantém mtime. Consumidores que inferem mudanças só por mtime precisam considerar hash/conteúdo; não alegar invalidação de cache baseada exclusivamente no relógio. Atime é restaurado na troca, mas leituras posteriores podem naturalmente atualizá-lo e ele não é usado na detecção de writers.

## Limites exatos

- Hashes/metadados conferidos na preparação e imediatamente antes da troca detectam conflitos observáveis, mas não são CAS do kernel. Existe uma janela entre a última verificação e rename/remove; não prometer exclusão mútua com processos externos arbitrários, nem proteção contra malware do mesmo usuário.
- Metadados são conferidos quando conhecidos no snapshot original. Em paths originalmente ausentes, o guard persistido é apenas hash/Missing; alterações externas só de metadata de um arquivo recém-criado não são distinguíveis sem persistir metadata planejada do post-state.
- Batch não é transação de filesystem. Preflight evita rollback parcial em conflitos já presentes; um writer surgido durante restauração pode interromper um lote após parte restaurada. Journal persistente e precondições permitem retomar sem substituir silenciosamente estados desconhecidos.
- Snapshots legados continuam legíveis. Sem guard, um arquivo alterado não é restaurado automaticamente: exige decisão/recovery explícita, mantendo journal. Arquivos já originais são aceitos.
- Windows: SACL/auditoria, owner/group de segurança, hardlink identity não são prometidos. Streams nomeados, atributos sparse/compressed/encrypted/offline/reparse e demais atributos especiais são detectados e recusados antes de mutação. Readonly Windows pode impedir rename do destino; nesse caso retorna erro e conserva bytes/metadados originais.
- Linux semantic inode flags (immutable/nodump/noatime/encryption/etc) e flags BSD não-zero no macOS são recusados, pois esta implementação não promete sua restauração. Flags Linux de layout EXTENTS/INLINE_DATA/INDEX não são políticas de acesso e podem variar na nova alocação.
- Metadados nativos só são restaurados na plataforma correspondente. Xattrs/ACLs têm limite conservador de 1 MiB; arquivo com propriedades sem leitura/restauração autorizada falha antes do rename.
- Unix/macOS ainda exigem execução do CI nativo; branches condicionais não constituem evidência de runtime.

## Evidência e arquivos

TDD: root confirmou RED real do teste `atomic_write_preserves_modified_time_root_and_nested`: timestamp atual versus timestamp de 2020, Cargo exit 1. Regressões foram escritas antes do fix de timestamps. Os demais testes ampliam proteção para binary rollback, writer externo, preflight de lote, delete/recreate, readonly, executable bit, native xattrs/ACL e rejeição ADS.

Arquivos alterados por A:

- `crates/domain/src/lib.rs` — FileSnapshot e VOs puros;
- `crates/infrastructure/src/files.rs` — implementação por handles, preconditions, metadados nativos, unit regressions;
- `crates/infrastructure/src/storage.rs` — somente SnapshotDto, conversões/save/decoding e testes DTO;
- `crates/infrastructure/tests/files.rs` — regressões de filesystem;
- `docs/readiness/report-files.md` — este relatório.

Rustfmt focado passou nos três arquivos owned não compartilhados; `storage.rs` fica para fmt central. Nenhum Cargo build amplo/commit executado por A. GREEN e gates finais são centralizados pelo root e devem ser acrescentados com resultados reais antes de declarar candidato completo.
