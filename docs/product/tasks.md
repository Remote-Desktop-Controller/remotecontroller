# Tasks e status — produto completo

Atualizado em 2026-10-05. Base ba5ee3a, branch codex/remote-product.

| Task | Peso | Estado atual | Evidência e próximo gate |
|---|---:|---|---|
| T00 Base local, integridade e instalação | 40 | Verificado | Suíte v0.3 e instalação core passaram; release pertence ao gate final |
| T01 Serviço remoto e MCP | 15 | Verificado | HTTP real e container com persistência passaram |
| T02 Contas, OAuth e pareamento | 10 | Verificado | PKCE, isolamento, revogação e formulários com cookies passaram |
| T03 Agente e recuperação | 10 | Verificado | Fluxo real IPC2, saturação/outbox e crash do daemon passaram |
| T04 Memória por sessão e logs | 8 | Verificado | Migração/restart/spool/notas e isolamento passaram |
| T05 Arquivos e terminal | 7 | Verificado com limite documentado | Terminal e arquivos passaram; Unix exige escritores cooperativos |
| T06 Instalação, hospedagem e plugin | 5 | Verificado | Pacote/instalador, autostart, plugin/schema/ícone e infraestrutura preparados |
| T07 Revisão e gates finais | 4 | Verificado | CI por SO, browser/container, release instalado, benchmarks e hashes passaram |
| T08 Trabalho manual do proprietário | 1 | Última tarefa, aguardando engenharia | Domínio/host/callback, identidade/políticas, conexão real ChatGPT |

Os pesos são convenção de planejamento, não estimativa de horas. Os 40 pontos do marco anterior não comprovam esta nova versão. Não se atribui 99% enquanto testes, instalação e distribuição automática permanecem pendentes. `status.json` é o ledger legível por máquina; `tools/product-status.ps1` computa somente marcos verificados.

Até três agentes existentes foram usados em superfícies independentes; nenhum agente extra foi criado para a revisão/correção. Builds são serializados com dois jobs. Logs de tentativas falhas permanecem distinguíveis das validações posteriores. Nenhum defeito de engenharia será transferido para T08.

Prontidão técnica ponderada atual: 99/100. Todos os marcos automáticos foram verificados. Produto público validado: false. A única tarefa de proprietário permanece T08, por último.
