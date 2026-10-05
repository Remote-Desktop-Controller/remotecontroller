# Auditoria de conclusão autônoma

Produto 0.3.0, protocolo IPC 2. Fonte do produto validada: 4bf5884; teste de observação Windows corrigido em a6596e5. A diferença não altera src/Cargo/migrations/deployment do produto. Evidências detalhadas em validation.md, status.json e runs vinculados.

| Requisito | Evidência autoritativa | Estado |
|---|---|---|
| Runtime Rust existente, dados v0.2 e recuperação | migrations 002/003; suites storage/sessions/fullstack/lifecycle; release instalado | Verificado |
| Conta/cookie/CSRF, autenticação e revogação | remote-service/tests/flows.rs; Chromium nativo e CI container | Verificado |
| MCP HTTP/OAuth/PKCE/resource/scopes/replay | fluxos HTTP com duas contas, refresh/revoke e agente real | Verificado |
| Código de pairing, expiração, segredo de polling, entrega única | testes flows e remote_flow; aprovação por formulário real | Verificado |
| Ownership dispositivo/workspace e envelopes | testes duas contas/relay e validação IPC | Verificado |
| Agente, presença, reconexão, outbox e cancelamento sob carga | runner_regressions; remote_flow; crash recovery do daemon próprio | Verificado |
| Sessões create/list/resume/close, notas e retomada | nove testes sessions, incluindo migração, restart e isolamento | Verificado |
| Índice de inputs/outputs/tools, grafo e consulta com limites | store/operations/events e context/process suites; referências raw/offsets | Verificado |
| Arquivos mkdir/move/remove e journal | mutations e recuperação/conflictos; segurança documentada | Verificado com escritores cooperativos em Unix |
| Terminal PTY/ConPTY, stdin, resize, quotas, árvore e spool | testes nativos em cada SO; aprovação local e fingerprint | Verificado |
| Instalar/parear/iniciar/parar/autostart/diagnóstico | instalação core e produto reais; perfil preservado; registro removido | Verificado local e CI Windows |
| Container, persistência e preparação HTTPS | compose/build/non-root/restart/login e browser CI | Verificado; certificado público depende de domínio real |
| Plugin/skills/ícone/casos/revisão/URLs configuráveis | schema oficial, smoke ZIP, SVG512 e DemoRecordingUrl | Preparado e validado; identidade/políticas/demo reais dependem do proprietário |
| fmt/check/Clippy/test/bench/release e pacote por SO | Linux/macOS/container 37309996209; Windows 37313011274 | Verificado: 37313011274 Windows; 37309996209 Linux/macOS/container |
| Artefatos/checksums finais | package-product.ps1 e hashes internos/ZIP | Verificado: três ZIPs/12 hashes internos por pacote em artifacts.json |
| Domínio/conta real/ChatGPT web-móvel/publicação/assinatura quando exigida | manual-owner.md | T08 manual; não validado publicamente |

O percentual é uma convenção de marcos: 99 automáticos verificados e um manual. Não representa fração de tempo nem aprovação da plataforma. Produto público validado permanece false. Não há treinamento de modelo, contexto infinito, deploy público ou assinatura afirmados sem prova.

Limites aceitos: sessões/histórico completos ficam no PC; relay mantém resultados limitados para continuidade/transporte. Sem e-mail/reset de senha nesta versão; guardar credenciais. Terminal aprovado tem privilégios do usuário e não é sandbox. Corrida de troca externa de nome em Unix pode produzir conflito após efeito e exigir reconciliação local; não é eliminada nem transferida à tarefa manual de publicação.
