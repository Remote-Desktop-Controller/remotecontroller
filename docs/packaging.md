# Distribuição

Entregar os dois executáveis local-daemon e mcp-gateway juntos, com README,
licença e exemplo de configuração MCP. LibSQL/libgit2/tree-sitter são vinculados
no build; Rust/C/CMake são ferramentas do desenvolvedor, não do usuário final.

CI define matrix Windows/Linux/macOS, executa gates e prepara ZIPs/checksums com
`tools/package.ps1` (PowerShell 7 nos runners). O CI final v0.2 passou nos três OS:
[run 36880261171](https://github.com/Remote-Desktop-Controller/remotecontroller/actions/runs/36880261171).
Evidências atuais: `readiness/progress.md`.

A [release v0.2.0](https://github.com/Remote-Desktop-Controller/remotecontroller/releases/tag/v0.2.0)
publica Windows x64 GNU validado localmente, Linux x64 e macOS arm64 gerados pelo
CI. Os três ZIPs e cada executable em seus manifests foram conferidos por SHA-256.

Windows GNU pode depender das DLLs de runtime MinGW conforme link final.
Inspecionar imports e incluir runtime necessário ou adotar build MSVC na release.
Neste build, objdump verificou somente DLLs Windows/UCRT em ambos os executáveis
de desenvolvimento; não houve import de DLL externa MinGW. O inventário final
GNU v0.2 também confirmou somente Windows/UCRT:
[release-imports-final-v0.2](evidence/release-imports-final-v0.2.log).
Não distribuir token, banco, cargo registry, target inteiro ou config com paths do
desenvolvedor. O CLI instala por usuário e verifica manifest SHA-256 antes/depois
da cópia; atualização local mantém versão anterior e troca ativação atomicamente.
Assinatura pública/notarização dependem de credenciais externas ainda ausentes.

State-dir é por usuário/workspace. O host MCP aponta a `local-daemon connect`,
que verifica ou inicia o daemon e conecta o gateway stdio. `register-host` grava
configuração explícita de Codex/Claude/generic com backup e sem substituir entrada
existente. Use uma pasta permanente para os binários extraídos. Nenhum processo abre porta TCP.
Cloud/auth/device licensing podem ser adicionados por adapters opcionais fora do
caminho crítico; nenhum código ou terminal é enviado por default.
