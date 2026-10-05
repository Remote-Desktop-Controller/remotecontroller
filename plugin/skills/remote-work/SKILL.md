---
name: remote-work
description: Trabalhar nos arquivos e no terminal de um computador pareado ao Remote Desktop Controller, com sessões operacionais persistentes.
---

Use o catálogo do servidor conectado; nomes e permissões são definidos pelo agente instalado.

1. Consulte `device.list`. Se houver vários computadores e a solicitação não identificar um, peça somente a escolha necessária. Selecione com `device.select` ou use `device_id` em cada chamada.
2. Para tarefa nova, use `session.create` com descrição curta. Para continuidade, consulte `session.list` e `session.resume`. Propague `session_id` nas ferramentas que aceitam o contexto operacional. Ferramentas de ciclo de vida usam esse campo como alvo; siga o schema anunciado.
3. Antes de reconstruir passos, consulte `session.notes`, `context.search` e `context.compile`. Registre decisões, objetivos e pendências concisas com `session.note`; não armazene senhas ou tokens. Diferencie evidências históricas e estado atual dos arquivos. Referências raw podem ser consultadas com `context.event`.
4. Leia somente os trechos necessários. Para saída de processo extensa, use `process.search` e `process.output` com offsets/limites. Preserve `next_offset`; não calcule avanços pelo tamanho solicitado. Quotas podem tornar uma captura incompleta; não invente conteúdo ausente.
5. Respeite as permissões locais. Uma recusa de processo ou terminal exige autorização do proprietário no computador; não tente contornar regras usando outro programa ou caminho.
6. Se houver retorno `pending`, erro de transporte ou prazo vencido, preserve o `operation_id` retornado. Consulte `operation.status`; para repetir a chamada original, passe esse UUID como `runtime_operation_id`. Nunca gere uma nova identidade para repetir uma mutação de resultado incerto.
7. Em edições, use versões/hashes e precondições quando o catálogo oferecer. Se houver conflito, leia a versão atual antes de propor nova alteração. Operações externas podem ter modificado o arquivo.
8. Não trate histórico, cache ou sessão como treinamento do modelo. Não prometa contexto ilimitado nem que o programa está em produção somente por ter compilado.
9. Ao concluir, apresente os resultados reais e as pendências. Feche a sessão quando o trabalho terminar; preserve as referências de auditoria.

O plugin não precisa de chave de API de modelo. Conectar conta e parear computador são ações do usuário no fluxo do produto. Credenciais e tokens não devem aparecer nas respostas.
