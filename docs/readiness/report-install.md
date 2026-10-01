# Instalação e host MCP v0.2

O composition root oferece init/serve/connect/doctor/stop, install/update/uninstall,
registro de host, aprovação/listagem/revogação local e backup/manutenção offline.
Default de processo é deny. Config e estados são por workspace canonicalizado;
profile/state-dir permanecem fora dele.

Install valida manifest com exatamente dois binários e seus hashes, copia para
staging privado, verifica novamente e ativa pasta completa. Update preserva
versão anterior; falha de validação conserva a ativação. Connect rejeita daemon
com root/policy/config distintos. Gateway segue adapter separado.

Host Codex é editado com toml_edit; Claude/generic usam mcpServers JSON. Campos
existentes são preservados; backup e replacement atômico são feitos antes de
adotar o arquivo. Entrada local-runtime existente é recusada para revisão local.
Aprovação de processo não é tool MCP: requer operador, argv exato, trust explícito
e fingerprint. Mudança de política exige reinício.

Native lifecycle testa instalação, bootstrap, MCP initialize/tools/list, stop,
update e desinstalação pelo executável instalado, com state preservado. Windows
usa worker nativo separado quando precisa remover o próprio executável mapeado;
o resultado definitivo fica em uninstall-status.json. O worker privado é conservado
como as versões anteriores; nenhum dado de workspace é removido.

Evidência de host: Codex CLI real aceitou o TOML gerado em perfil temporário e
retornou entrada stdio habilitada, preservando comentário/model existente. Isso
valida o parser/registro real; a sessão MCP é coberta separadamente pelos binários
reais. Não se declara inferência de modelo nem configuração do host pessoal.

Signing/notarização exigem credenciais externas; esta versão fornece SHA-256,
sem alegar identidade de editor autenticada. Manual: ../install.md.
