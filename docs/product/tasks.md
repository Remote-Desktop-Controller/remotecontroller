# Tasks e status — produto completo

Atualizado em 2026-10-04. Base ba5ee3a, branch codex/remote-product.

| Task | Peso | Estado atual | Evidência e próximo gate |
|---|---:|---|---|
| T00 Base local, integridade e instalação | 40 | Validação integrada | Baseline anterior preservado; corrigir/revalidar ciclo instalado |
| T01 Serviço remoto e MCP | 15 | Implementado; validação | HTTP real; suíte integral e container pendentes |
| T02 Contas, OAuth e pareamento | 10 | Implementado; validação | PKCE, isolamento, revogação e formulários com cookies |
| T03 Agente e recuperação | 10 | Revisão e correções | Fluxo real passou antes do IPC2; saturação/outbox em correção |
| T04 Memória por sessão e logs | 8 | Implementado; validação | Migração/restart/spool/notas passaram; novo isolamento pendente |
| T05 Arquivos e terminal | 7 | Validação | 3 testes nativos de terminal passaram; limites POSIX em revisão |
| T06 Instalação, hospedagem e plugin | 5 | Em andamento | Container e manifestos preparados; pacote/instalador em execução |
| T07 Revisão e gates finais | 4 | Em andamento | Revisor independente; builds seriais; CI e release pendentes |
| T08 Trabalho manual do proprietário | 1 | Última tarefa, aguardando engenharia | Domínio/host/callback, identidade/políticas, conexão real ChatGPT |

Os pesos são convenção de planejamento, não estimativa de horas. Os 40 pontos do marco anterior não comprovam esta nova versão. Não se atribui 99% enquanto testes, instalação e distribuição automática permanecem pendentes. `status.json` é o ledger legível por máquina; `tools/product-status.ps1` computa somente marcos verificados.

Até três agentes existentes foram usados em superfícies independentes; nenhum agente extra foi criado para a revisão/correção. Builds são serializados com dois jobs. Logs de tentativas falhas permanecem distinguíveis das validações posteriores. Nenhum defeito de engenharia será transferido para T08.
