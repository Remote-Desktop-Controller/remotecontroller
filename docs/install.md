# Instalar e conectar

Extraia o pacote em uma pasta permanente. Ele contém os dois executáveis e
`manifest.json`; a execução não requer Rust, Node, Python ou Docker. Exemplos
Windows abaixo; Linux/macOS usam `./local-daemon` e caminhos nativos.
Após extrair em Unix, execute `chmod +x local-daemon mcp-gateway`.

```powershell
.\local-daemon.exe install --source . --workspace "C:\meu-projeto"
.\local-daemon.exe init --workspace "C:\meu-projeto"
.\local-daemon.exe register-host --host codex --config-file "$env:USERPROFILE\.codex\config.toml" --workspace "C:\meu-projeto" --allow-write
```

Reabra o host MCP. A entrada `local-runtime` inicia `connect`, que verifica o
daemon existente ou inicia um automaticamente, depois liga o gateway stdio.
`--allow-write` concede escrita; sem ele a conexão permite leitura e GitRead.
O registro preserva outras configurações e cria backup. Uma entrada já existente
é recusada para evitar substituir uma integração sem revisão. O executável usado
no registro deve permanecer disponível. Ele segue a versão ativada no perfil.
Para outros hosts, `--host claude` ou `generic` escreve JSON `mcpServers`.

Perfil padrão: `%USERPROFILE%\.local-runtime` ou `$HOME/.local-runtime`.
`--profile` escolhe outro local. Estado/configuração são separados por workspace
canonicalizado. O perfil deve ficar fora do projeto. Use os mesmos argumentos de
workspace, perfil e permissões nos comandos de diagnóstico e conexão.

```powershell
.\local-daemon.exe doctor --workspace "C:\meu-projeto" --allow-write
.\local-daemon.exe stop --workspace "C:\meu-projeto"
.\local-daemon.exe update --source "C:\novo-pacote" --workspace "C:\meu-projeto"
```

Pare o daemon antes de atualizar; conecte novamente depois. A atualização verifica
SHA-256 antes e depois da cópia e ativa uma pasta imutável completa. Conserva a
versão anterior e o estado. Hashes detectam corrupção; esta entrega não possui
assinatura pública de editor. Compare o checksum com a origem do pacote.

Processos são negados por padrão. Só o operador local aprova um programa absoluto
e seus argumentos exatos, com pin de SHA-256:

```powershell
.\local-daemon.exe policy approve --workspace "C:\meu-projeto" --program "C:\Tools\programa.exe" --trust-unconfined -- argumento1 argumento2
.\local-daemon.exe policy list --workspace "C:\meu-projeto"
.\local-daemon.exe policy revoke --workspace "C:\meu-projeto" --program "C:\Tools\programa.exe" -- argumento1 argumento2
```

Para executar regras aprovadas, registre a conexão também com `--allow-process
--allow-process-outside-workspace --allow-process-network`. Esses programas têm
acesso não confinado ao sistema e à rede; não há sandbox de OS. Uma mudança no
binário exige nova aprovação. Pare e reinicie o daemon após mudar a política.
O MCP não oferece ferramenta de aprovação.

Logs privados ficam em `states/<workspace>/daemon.log`, com rotação de 1 MiB.
Saída dos processos fica em `process-spool`, com limites configuráveis; respostas
informam truncamento e captura incompleta. `doctor` mostra política, workspace,
configuração e métricas. Se falhar, consulte o log da pasta de estado.
Erros de startup ficam em `bootstrap.log`. No Unix, perfis cujo caminho exceda
o limite do socket usam um endpoint curto em `/tmp/rdc-<hash-do-estado>/daemon.sock`,
com diretório privado; banco e segredo permanecem na pasta de estado original.

Com daemon parado, faça backup ou simule retenção:

```powershell
.\local-daemon.exe backup "C:\backups\runtime.db" --workspace "C:\meu-projeto"
.\local-daemon.exe maintain --workspace "C:\meu-projeto"
.\local-daemon.exe maintain --workspace "C:\meu-projeto" --apply --backup "C:\backups\antes-retencao.db"
.\local-daemon.exe uninstall --workspace "C:\meu-projeto"
```

Retenção usa um corte de idade, limite de registros e backup novo obrigatório.
Remove checkpoints já revertidos e spools de processos encerrados elegíveis;
conserva o grafo e as linhas históricas. Não remove arquivos do projeto. Backup
inclui DB consistente e uma pasta `.spool`. Desinstalar remove somente a versão
ativa verificada; conserva estado, backups, versões anteriores e registro do host.
Remova a entrada do host se não for voltar a usar a integração.

No Windows, ao desinstalar pelo executável ativo, um worker nativo separado
aguarda sua saída e registra o resultado em `uninstall-status.json`. O worker
privado permanece junto às versões conservadas. Pare todos os daemons desse
perfil antes de desinstalar; um executável em uso impede sua remoção.
