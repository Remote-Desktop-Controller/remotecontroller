# Task05 — arquivos e terminal interativo

Implementação 2026-10-04 em revisão e validação coordenadas pelo root. Este relatório descreve código local; não declara CI/daemon remoto/ChatGPT validado.

## Arquivos

Tools `filesystem.mkdir`, `filesystem.move` e `filesystem.remove`. Todas exigem WriteWorkspace. mkdir cria um único diretório em pai existente; move aceita arquivo regular, destino inexistente e pais existentes; remove usa o journal/checkpoints de batch-edit já existente. Não há remoção recursiva nem move de árvores.

Migração 003 adiciona `filesystem_journal` independente de FileSnapshot para representar diretórios corretamente. Uma staging directory reservada `.rdc-<operation>.directory` é criada e sua identidade obtida antes de preparar journal; a exposição no path público só ocorre depois do journal durável. Crash anterior ao journal pode deixar staging excluída das ferramentas; não se apaga staging desconhecida sem prova de propriedade.

Identidade nativa: volume/file index Windows, device/inode Unix. mkdir e move usam rename atômico sem substituir destino: Windows FILE_RENAME_INFO com ReplaceIfExists=false e RootDirectory handle fixado; Linux renameat2 RENAME_NOREPLACE; macOS renameatx_np RENAME_EXCL. O arquivo movido mantém o mesmo objeto e metadados/ACL. Paths usam validação existente e handles relativos; root, ignore policy, caminhos excluídos, symlink/namespace/traversal são rejeitados.

Recovery trata journals prepared antes de novas admissões. Só remove diretório de identidade correspondente e vazio; move revertido exige source ausente, destino com identidade e snapshot originais. Conteúdo externo ou paths ocupados causam conflito sem sobrescrever/apagar. Durante execução, rollback incerto conserva o journal e marca workspace poisoned; restart exige reconciliar esse conflito. Committed journals registram auditoria durável; `journal_id` é operation_id e retry volta ao resultado persistido.

## Terminal

`TerminalConfig` disabled por default, com programa absoluto, argv fixados e SHA256 local, até 16 terminais (default 4), input default 4 KiB (máximo 64 KiB), idle timeout default 900 s (máximo 24 h). `--allow-terminal` concede AllowInteractiveTerminal e é propagado pelo lifecycle/start/install. open/write/resize/close também exigem OutsideWorkspaceAccess e NetworkAccess; read exige ReadWorkspace. Config fica fora do workspace. A rejeição de shells genéricos em ProgramRule foi preservada.

Aprovação sem editar JSON: `local-daemon --workspace <root> --profile <profile> terminal approve --program <absolute> --trust-unconfined -- <argv...>`. Grava SHA256 e habilita somente config.terminal via replace_with_backup, preservando allowed_programs e demais limites. Após aprovar, reiniciar daemon com `--allow-terminal --allow-process-outside-workspace --allow-process-network`. `terminal revoke` desabilita a configuração e pede stop/restart. Teste CLI real prova pin/argv/preservação/revoke. device-agent delegará esse comando.

`portable-pty` 0.9 cria ConPTY/PTY reais. Só executa local-daemon `--terminal-guardian` com programa/argv/fingerprint aprovados. O helper confere fingerprint novamente. Windows associa um Job kill-on-close a si próprio antes de criar o shell; filhos herdam o Job. Um monitor do handle do processo pai encerra helper quando o pai morre. Unix preserva o grupo/sessão PTY, monitora parent PID e usa killpg; close também coleta ancestry viva com `/bin/ps` para jobs em grupos separados. Isso é gestão de árvore de shell confiável, não sandbox contra programas deliberadamente escapando/trocando de sessão.

`terminal.open` aceita cwd relativo e geometria, jamais programa/args remotos. UUID do terminal igual operation_id. `terminal.write` text limitado passa por fila limitada a oito entradas e uma thread por terminal; o Executor já fez claim idempotente antes da entrega, e repetir operation_id não reenvia input. Cancelamento depois de input enfileirado tem entrega potencialmente indeterminada, mas o ID permanece durável e não é executado de novo. `terminal.resize` usa API nativa; `terminal.close` pede fim da árvore; shutdown/Drop terminam os terminais. Readers e writers são threads dedicadas, fora dos workers database/Tokio. Não há shell em novas janelas Windows.

`terminal.read` retorna offsets e bytes exatos hex + texto explicitamente utf-8-lossy. PTY é stream único, sem separação stdout/stderr fictícia. Armazenamento usa clone do SpoolBudget de processos, incluindo mesmo limite global e teto de 4.096 registros. Tabela terminals persiste status/exit/operation/workspace; restart marca Running como interrupted e preserva spool. Captura perdida/quota é sinalizada. Nenhum PID persistido é usado para kill/reanexação após restart.

## Testes e estado de validação

- Infra integration `mutations.rs`: mkdir/move/remove, conteúdo, retries idempotentes, no-overwrite/root denial.
- Infra unit `fs_mutations`: crash após efeito antes de commit, rollback de identidade/diretório vazio, metadados do arquivo e preservação de alterações externas.
- Daemon integration `terminal.rs`: ConPTY/PTY real com helper `CARGO_BIN_EXE_local-daemon`, gravação real em arquivo, input retry sem duplicação, resize, quota input, filho background sem sobreviver ao close, spool reaberto, autorização/disabled defaults.
- Task03 novo teste scoped context permanece no mesmo conjunto sessions. Rustfmt executado.

Root executará `cargo test -p runtime-infrastructure --test sessions`, `--test mutations`, `--lib fs_mutations` e `cargo test -p local-daemon --test terminal`. `mutations-red.log` inicial registrou erro de compilação Clone DTO, corrigido; não foi falha comportamental do teste. Registrar GREEN e limitações reais aqui após resultados. CI Linux/macOS deve provar especialmente a árvore de jobs Unix e metadata/rename nativos.

Referências primárias: [API portable-pty](https://docs.rs/portable-pty/latest/portable_pty/) e [Windows FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info). RootDirectory suporta resolução relativa por handle; ReplaceIfExists=false devolve erro se destino existe.
