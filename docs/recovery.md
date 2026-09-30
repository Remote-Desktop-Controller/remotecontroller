# Recovery e retries

Inicialização: adquirir lock de state-dir e endpoint; abrir schema; registrar ou
retomar workspace; restaurar checkpoints prepared; reconciliar registros de
processos; finalizar operações Queued/Running interrompidas; só então aceitar
requests. Qualquer falha de restauração impede startup de sucesso.

Snapshot registra somente paths do lote e seus bytes anteriores ou inexistência.
Durante falha/cancelamento restaura todos esses paths. Novo lote começa depois
que lock de mutação é liberado. Rollback que falhe marca workspace como exigindo
recovery e impede tools subsequentes; o journal continua preparado.
O fence é relido sob o lock antes de uma mutação já admitida começar.
Cancelamento antes do commit restaura o lote. Depois de um commit bem-sucedido,
o resultado persistido continua Succeeded com seu checkpoint; cancelamento tardio
não apaga um efeito já confirmado. Erro de cleanup prevalece sobre Cancelled.

Resposta perdida não significa execução perdida. Operação executa separada da
conexão; retry com mesmo OperationId recupera resultado. Mesmo ID com outro tool,
input, workspace ou task retorna Conflict. Não repetir automaticamente operações
interrompidas por crash: elas terminam Failed com motivo de interrupção.

Processos são reconciliados como interrupted; PID isolado não comprova identidade
após restart. Job Object encerra filhos Windows quando o daemon perde o handle;
Unix process groups garantem cancelamento normal, mas não a limpeza automática
de todos os descendentes depois de SIGKILL no daemon. Essa diferença precisa de
validação/plataforma ou isolamento de sessão mais forte antes de produção Unix.

Teste recovery simula journal preparado + escrita parcial e reabre banco/kernel;
fullstack encerra abruptamente processo real do daemon e mantém gateway ativo.
Stress combina cancelamento, rollback e reabertura de todo o workspace temporário.
Não modificar arquivos reais do usuário para stress.
