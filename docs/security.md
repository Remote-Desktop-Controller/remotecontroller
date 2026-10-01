# Segurança e fronteira de confiança

Default: ReadWorkspace e GitRead. Escrita exige grant do operador. Processo
exige aprovação local, pin SHA-256 de executável absoluto e **vetor exato de argumentos**, sem
shell genérico. env_clear impede herança de segredos; SystemRoot é preservado no
Windows. Como não há sandbox de filesystem/rede para filhos, process.spawn
exige também OutsideWorkspaceAccess e NetworkAccess. O operador concede ambas
com `--allow-process-outside-workspace --allow-process-network`; só `--allow-process`
não as concede. Essa exigência conservadora deixa os efeitos explícitos na política.
SystemRoot permite iniciar programas nativos. Nenhuma autoelevação é implementada.

Filesystem usa um handle cap-std aberto para o root canonicalizado. Todas as
operações de workspace são relativas a esse handle, com validação de componentes,
../, namespace/ADS Windows, nomes de dispositivo, links e exclusões. cap-std
confina resolução mesmo quando uma symlink muda entre validação e open.
Arquivos .gitignore são leitura de política e não podem ser alterados por tools.
Exclusões padrão node_modules/target/.git/build/dist são configuráveis.

Banco, segredo, lock e socket ficam em state-dir fora do workspace. Unix usa
directory 0700/socket 0600/secret 0600. Windows usa DACL protegida com Owner
Rights e SYSTEM; Named Pipes rejeitam clientes remotos e primeira instância
impede dois listeners com o mesmo nome. Um lock de arquivo impede dois daemons
reconciliando o mesmo banco, mesmo com endpoints diferentes.

Segredo local de 32 bytes é requerido no handshake, comparado em tempo constante.
Seu caminho é passado ao gateway; conteúdo nunca é logado. Isso distingue
usuários locais, mas não pretende isolar um processo malicioso que já execute
com as mesmas credenciais do usuário e possa ler seus segredos. Administrador/SYSTEM
permanece fora desse modelo. Não existe bypass secreto nem autenticação cloud.

Processos: no Windows, CREATE_SUSPENDED permite associação ao Job Object com
KILL_ON_JOB_CLOSE antes de retomar a thread. No Unix, um guardian separado mantém
heartbeat por pipe; EOF após crash do daemon encerra o grupo do processo alvo.
Cancelamento fecha esse heartbeat e aguarda o guardian. Essas funções gerenciam
lifecycle; não são sandbox. Um descendente Unix que abandone deliberadamente o
grupo fica fora dessa garantia. Não matar PID persistido no restart: ele pode
ter sido reutilizado. Saída é sincronizada progressivamente em spool privado,
com quotas, buffers de cauda finitos e indicadores de truncamento/captura incompleta.

Limites reais: atomicidade entre múltiplos arquivos é obtida por journal/rollback,
não por transação do filesystem; writers externos devem ser coordenados com o
daemon. Queda de energia depende das garantias do filesystem/OS (fsync de arquivo,
e parent directory no Unix). ACLs não protegem contra o próprio usuário.
Git usa libgit2 sem hooks/executáveis, mas não constitui uma sandbox de parser
de repositórios hostis. Use somente metadados Git de origem confiável.
