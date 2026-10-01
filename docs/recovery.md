# Recovery e retries

Inicialização: adquirir lock de state-dir e endpoint; abrir schema; registrar ou
retomar workspace; restaurar checkpoints prepared; reconciliar registros de
processos; finalizar operações Queued/Running interrompidas; só então aceitar
requests. Qualquer falha de restauração impede startup de sucesso.

Snapshot registra paths, bytes anteriores ou inexistência, metadados suportados
e hash/Missing esperado depois da mutação. Durante falha/cancelamento restaura
esses paths somente se as precondições continuarem válidas. Mudança externa
desconhecida preserva o journal e impede sobrescrita. Novo lote começa depois
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
Unix usa guardian separado que monitora EOF no heartbeat do daemon e encerra
o grupo alvo; o próprio daemon não precisa executar cleanup depois de SIGKILL.
Descendentes que escapam deliberadamente do grupo não são confinados por esse
mecanismo. Os testes nativos precisam validar cada plataforma.

Teste recovery simula journal preparado + escrita parcial e reabre banco/kernel;
fullstack encerra abruptamente processo real do daemon e mantém gateway ativo.
Stress combina cancelamento, rollback e reabertura de todo o workspace temporário.
Não modificar arquivos reais do usuário para stress.

Backup e retenção são comandos locais com fence exclusivo: pare o daemon primeiro.
Backup usa VACUUM INTO, quick_check e cópia privada do spool, sem sobrescrever um
destino existente. Retenção tem dry-run; aplicar exige backup novo e intent audit
durável. Preserva prepared, processos ativos, grafo e arquivos do projeto.

Para restaurar um backup, mantenha o estado atual intacto e use uma **nova pasta
de estado fora do workspace**. Copie o DB de backup como `runtime.db` e sua pasta
`.spool` como `process-spool`; copie a configuração local desejada como `runtime.json`.
Inicie `serve` ou `connect` com o workspace original e `--state-dir` dessa pasta.
O daemon verifica journals antes de admitir requests e gera um segredo novo.
Atualize o registro do host para esse state-dir. Nunca substitua DB de daemon
ativo nem misture WAL/SHM do estado anterior com o backup.
