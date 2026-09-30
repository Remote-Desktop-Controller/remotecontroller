# remotecontroller

Runtime local em Rust para hosts MCP. O gateway stdio é um adapter; o daemon
executa as ferramentas, aplica políticas e persiste o histórico em libSQL.
Não existe serviço cloud, porta TCP ou dependência de Node/Python/Docker no uso.

Arquitetura e critérios de evolução: [RFC-0001](docs/rfcs/0001-runtime-local.md).

## Executar

Para usar o pacote nativo sem compilação, veja [instalar e conectar](docs/install.md).
`local-daemon connect` inicia o daemon e o gateway; `register-host` configura o
host MCP preservando suas configurações. Segurança e finalização v0.2:
[matriz de evidências](docs/readiness/progress.md).

Compilação de desenvolvimento requer Rust **1.98.1** e toolchain C/C++ (libSQL,
libgit2 e tree-sitter são compilados e incorporados aos binários).

```powershell
cargo build --workspace
.\target\debug\local-daemon.exe --workspace "C:\meu-projeto" --state-dir "C:\Users\Notebook\AppData\Local\LocalRuntime\meu-projeto" --endpoint "\\.\pipe\local-runtime-demo" --allow-write
```

Em outro processo, o host MCP inicia:

```powershell
.\target\debug\mcp-gateway.exe --endpoint "\\.\pipe\local-runtime-demo" --token-file "C:\Users\Notebook\AppData\Local\LocalRuntime\meu-projeto\ipc.secret"
```

No Linux/macOS, use os mesmos argumentos com paths nativos; endpoint é um Unix
socket dentro da pasta privada de estado. Sem `--endpoint`, o daemon escolhe um
nome determinístico a partir do state-dir (Windows) ou `state-dir/daemon.sock`.
O log `daemon ready` informa o endpoint. A pasta de estado deve ficar **fora**
do workspace. Um daemon controla um workspace por execução; várias instâncias
usam pastas de estado distintas. Abrir novamente a mesma pasta recupera seu ID.

O default concede leitura e GitRead. Escrita precisa de `--allow-write`.
Processos precisam de autorização explícita e regras de programa/argumentos no
JSON de `--config`; shells genéricos são rejeitados. Leia [security](docs/security.md)
e conceda `--allow-process-outside-workspace --allow-process-network` apenas para
programas confiáveis: o gerenciador de processos não é uma sandbox do OS.
antes de conceder essa capability. Argumentos, ambiente e cwd não são strings de shell.

Configuração mínima opcional:

```json
{"scheduler":{"queue":128,"file_reads":16,"file_writes":2,"processes":4,"cpu":2},"operation_timeout_ms":60000,"cache_capacity":256,"cache_ttl_seconds":10800}
```

O MCP `_meta.runtimeOperationId` aceita um UUID escolhido pelo host para retries
entre chamadas MCP independentes. Retries IPC preservam automaticamente o ID.
Resposta de execução contém `operation_id`, `status`, `result` e `error`.

## Ferramentas

| Grupo | Tools |
|---|---|
| Filesystem | read, read_range, list, search, stat, write, patch, batch_edit |
| Processos | spawn, status, stdout, stderr, kill |
| Git | status, diff, show |
| Workspace | info |
| Contexto | recent, causal, compile |
| Operações | status, cancel |
| Checkpoints | create, rollback |
| AST | code.inspect, code.symbols (Rust/Python, queries tree-sitter) |

Nomes completos têm prefixo, por exemplo `filesystem.read`. Listagem e busca
respeitam .gitignore e exclusões configuráveis. Escritas são UTF-8; patch exige
uma ocorrência única e verifica o hash da versão lida. Lotes aceitam
`{"edits":[{"path":"a.txt","content":"novo"}]}`; `content:null` remove arquivo.
Todos os paths são relativos ao workspace. Pais devem existir.

## Verificação

```text
cargo fmt --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace
cargo test --workspace
cargo bench --no-run
cargo bench -p runtime-infrastructure --bench runtime
```

O build anterior ao teste fornece os dois binários para o teste fullstack.
Stress padrão usa somente tempdirs: 10.000 arquivos, 1.000 edições,
cancelamento, novo lote, rollback, restart e validação de todos os conteúdos.
O teste `sleeper` ignorado é um fixture de processo filho, invocado pelos testes
de cancelamento/timeout; não é um teste pendente de produto.

## Estrutura

```text
crates/domain          IDs, entidades, estados e políticas puras
crates/ports           contratos pequenos de repositórios/tools/OS
crates/application     registry, scheduler, execução e cancelamento
crates/protocol        envelopes/DTOs versionados
crates/transport       framing, Named Pipes/UDS, autenticação e retry
crates/infrastructure  libSQL, Moka, filesystem, processos, AST, Git, kernel
apps/local-daemon      composition root e CLI
apps/mcp-gateway       rmcp + stdio; depende apenas de protocolo/transporte
migrations/            schema versionado
docs/                  arquitetura, segurança, operação e ADRs
```

Consulte [arquitetura](docs/architecture.md), [recovery](docs/recovery.md),
[SDK de tools](docs/tool-sdk.md) e [benchmarks](docs/benchmarking.md).
Evidências e limites da entrega ficam em [validation](docs/validation.md).
