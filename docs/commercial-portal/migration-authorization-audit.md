# Auditoria estática da migration de autorização

Nenhuma consulta, migration ou teste SQL executado nesta revisão. Nenhum deploy,
mudança de configuração ou operação financeira. A correção está somente nos arquivos locais.

## SQL anterior

O alerta de operação destrutiva corresponde a
`DROP FUNCTION public.portal_authorize(uuid,uuid,text,boolean)`.
O detector público atual do Supabase Studio verifica DROP/DELETE/TRUNCATE e
variantes dinâmicas; REVOKE/GRANT não são o gatilho dessa mensagem.

DROP FUNCTION elimina a função, seu OID e suas permissões próprias. Não elimina
linhas de pedidos, clientes, pagamentos ou direitos. Sem CASCADE, dependências
registradas impedem a remoção em vez de serem removidas junto. Criar a assinatura
de cinco parâmetros com DEFAULT NULL permitia chamadas de quatro argumentos,
mas não preservava a identidade da função original nem suas dependências.

CREATE FUNCTION instala o corpo; os UPDATE/INSERT dentro dele só são executados
em chamadas futuras, conforme modo/estado. Instalar o SQL não ativa direitos nem
inicia os 30 dias. Não há DELETE, TRUNCATE, DROP TABLE, ALTER TABLE, CREATE POLICY,
ALTER POLICY ou DROP POLICY na migration. SECURITY DEFINER já existia na função
anterior e exige que a execução continue restrita ao servidor.

REVOKE restringe somente a execução da nova função para PUBLIC/anon/authenticated;
GRANT autoriza service_role. Não muda permissões das tabelas nem políticas RLS.
NOTIFY pede atualização do cache PostgREST. BEGIN/COMMIT tornam o conjunto
transacional. O arquivo anterior tinha COMMIT; o trecho colado no chat não tinha.

## Versão corrigida

- Preserva integralmente a função antiga de quatro argumentos, seu OID, corpo,
  proprietário, permissões e dependências. Não há DROP nem substituição de função.
- Cria uma assinatura adicional de cinco argumentos, todos obrigatórios. Não
  usa DEFAULT, evitando ambiguidade entre as assinaturas de quatro/cinco argumentos.
- A nova função rejeita lista NULL; lista vazia não autoriza direito algum. O
  portal publicado já envia p_verified_orders sempre, inclusive lista vazia.
- Confere existência/tipos das colunas utilizadas, RLS habilitada nas quatro
  tabelas referenciadas, funções-base/retorno JSONB e privilégios esperados da
  função antiga antes de criar o novo objeto.
- Se a função de cinco argumentos já existir, aborta e não a sobrescreve. Não é
  um script para reaplicar cegamente: nesse caso, auditar o estado instalado.
- REVOKE/GRANT atingem exclusivamente a nova assinatura. Dados, tabelas,
  índices, constraints, políticas e funções preexistentes não são alterados.
- Limites locais de lock/statement evitam espera prolongada. Qualquer erro deixa
  a transação abortada; nenhuma alteração parcial pode ser confirmada.

Compatibilidade conferida estaticamente contra 202610080001_commercial.sql e
Billing.authorize do repositório. Não foi consultado o catálogo de produção;
portanto esta revisão não atesta divergências manuais, triggers DDL adicionais ou
o estado instalado ao vivo. Os pré-requisitos do script são verificações de
compatibilidade, não uma certificação completa de todo o banco.

Recomendação: não executar o trecho antigo/incompleto. A versão local corrigida
pode ser executada integralmente, de BEGIN até COMMIT, no Supabase exclusivo do
portal com a migration-base instalada. Ela não contém comandos para apagar ou
atualizar registros existentes durante a instalação. Se os pré-requisitos
recusarem o banco, interromper e investigar o erro; não remover as verificações.

Essa versão nova não foi executada nem submetida à suíte SQL, conforme a instrução
do usuário. Os 80 testes da revisão anterior não certificam este novo script.

## Referências primárias

- https://github.com/supabase/supabase/blob/master/apps/studio/components/interfaces/SQLEditor/SQLEditor.constants.ts
- https://github.com/supabase/supabase/blob/master/apps/studio/components/interfaces/SQLEditor/SQLEditor.utils.ts
- https://www.postgresql.org/docs/current/sql-createfunction.html
- https://www.postgresql.org/docs/current/sql-dropfunction.html
- https://www.postgresql.org/docs/current/sql-begin.html
