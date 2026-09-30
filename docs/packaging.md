# Distribuição

Entregar os dois executáveis local-daemon e mcp-gateway juntos, com README,
licença e exemplo de configuração MCP. LibSQL/libgit2/tree-sitter são vinculados
no build; Rust/C/CMake são ferramentas do desenvolvedor, não do usuário final.

CI define matrix Windows/Linux/macOS, executa gates e prepara artifacts release.
CI ainda precisa rodar num remoto Git configurado; definir workflow não comprova
compatibilidade nem publicação. Esta sessão valida o host Windows GNU.

Windows GNU pode depender das DLLs de runtime MinGW conforme link final.
Inspecionar imports e incluir runtime necessário ou adotar build MSVC na release.
Neste build, objdump verificou somente DLLs Windows/UCRT em ambos os executáveis
de desenvolvimento; não houve import de DLL externa MinGW. O mesmo inventário
deve acompanhar os binários finais.
Não distribuir token, banco, cargo registry, target inteiro ou config com paths do
desenvolvedor. Assinatura, instalador, notarização e auto-update não estão implementados.

State-dir é por usuário/workspace. O host MCP aponta ao gateway com endpoint e
token-file; o daemon é iniciado separadamente. Nenhum processo abre porta TCP.
Cloud/auth/device licensing podem ser adicionados por adapters opcionais fora do
caminho crítico; nenhum código ou terminal é enviado por default.
