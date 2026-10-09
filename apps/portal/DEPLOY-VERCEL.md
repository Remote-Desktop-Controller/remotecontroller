# Deploy do portal pela Vercel CLI, sem integração Git

Preparação verificada em 09/10/2026. Branch de trabalho: `feat/commercial-portal`. Diretório de upload: **somente `apps/portal`**. O repositório privado continua na organização; não é necessário importar/conectar o GitHub ao projeto Vercel.

## Publicação concluída em 09/10/2026

- Portal público: https://remotecontroller-portal.vercel.app
- Deployment Production READY: `dpl_ZCivbDcWQpsw1Gzk3RjRE6jYkrsu`, código `84d4504`.
- Webhook POST público: https://remotecontroller-portal.vercel.app/api/asaas/webhook
- Production usa Asaas de produção, com `SALES_ENABLED=false` e `REMOTE_SERVICE_AVAILABLE=false`, confirmados por HTTP na aplicação publicada.
- Nenhuma cobrança real foi criada. O domínio do blog e a produção MCP não foram modificados.
- Build e typecheck locais/remotos passaram; 58 testes locais passaram. Lint: zero erros, três advertências existentes.
- Dez páginas verificadas por HTTP 200; navegador descartável validou landing desktop, login/cadastro mobile, ausência de overflow/erros JavaScript e proteção do dashboard.
- A rota rejeita chamadas sem token com 401. Checagem GET autenticada validou Supabase e a API Asaas por leitura; ela retorna somente indicadores de saúde, sem pagamentos, chaves ou tokens. POST autenticado sem pagamento confirmou o processamento da rota. POST de pagamento inexistente falhou sem conceder direito.
- Todos os eventos solicitados têm regressões locais: RECEIVED, CONFIRMED, REFUNDED, PARTIALLY_REFUNDED e DELETED. A notificação nunca substitui a consulta à API; Pix CONFIRMED aguarda RECEIVED, e somente uso remoto autorizado inicia o período.
- Nenhuma variável obrigatória do portal/webhook está ausente. `DELIVERY_DEADLINE`, `SUPPORT_EMAIL`, `LEGAL_OPERATOR_NAME` e `REFUND_POLICY` ainda precisam ser definidos antes de abrir vendas reais.

### Campos exatos do webhook no painel Asaas

| Campo | Preencher |
| --- | --- |
| Nome | `Remote Controller - Pagamentos` |
| URL | `https://remotecontroller-portal.vercel.app/api/asaas/webhook` |
| Ativo | Sim |
| Versão da API | v3 |
| Tipo de envio | Não sequencial (`NON_SEQUENTIALLY`) |
| Fila | Ativada / não interrompida |
| Token de autenticação | Copiar o valor de `.vercel/asaas-webhook-token.txt` neste checkout; não copiar o nome da variável nem a chave API Asaas |
| Eventos | `PAYMENT_RECEIVED`, `PAYMENT_CONFIRMED`, `PAYMENT_REFUNDED`, `PAYMENT_DELETED`; se disponível, também `PAYMENT_PARTIALLY_REFUNDED` |

O token foi criado aleatoriamente, salvo como Secret na Vercel e em arquivo local com ACL restrita ao usuário/SYSTEM, ignorado por Git e upload. Não imprima o valor em logs ou mensagens. O painel envia esse valor no header `asaas-access-token`.

O portal está pronto para receber notificações reais autenticadas. A configuração do webhook no painel Asaas é uma ação manual; esta execução não declarou que ela já foi salva nem realizou pagamento/reembolso real para gerar um evento financeiro.

As seções abaixo preservam o procedimento inicial; a etapa antiga de publicação interrompida foi superada pela validação acima.

## Projeto já preparado

- Conta autenticada na CLI: `andrefonn`.
- Escopo pessoal: `andrefonns-projects` (Hobby).
- Projeto exclusivo: `remotecontroller-portal`.
- Framework Next.js; Node.js 24.x; Root Directory vazio, porque o diretório enviado pela CLI já é `apps/portal`.
- URL padrão reservada pela Vercel: `https://remotecontroller-portal.vercel.app`. A reserva não significa que já há um deployment publicado.
- A API da Vercel confirmou ausência de `link`/`gitRepository` no projeto.
- O blog/projeto `toemdia` e seu domínio não foram modificados.

O primeiro `vercel link` criou o projeto e tentou conectar o Git automaticamente. A tentativa foi recusada pelo plano Hobby, mas o projeto e o vínculo local foram criados. Nenhuma conexão Git ficou estabelecida. Agora os comandos usam esse projeto existente e enviam os arquivos locais.

A CLI 63.1.0 foi instalada globalmente. `.vercel/project.json` guarda o vínculo local e é ignorado pelo Git e pelo upload. O `.env.local` criado automaticamente pela CLI também é ignorado; não copie seu conteúdo para mensagens ou commits.

## Configurar as variáveis que faltam

Abra [Environment Variables do projeto pessoal](https://vercel.com/andrefonns-projects/remotecontroller-portal/settings/environment-variables). Configure o ambiente **Production desse projeto novo**. Isso não altera a branch main, não conecta Git e não altera o blog. A cobrança permanecerá em sandbox, sem vendas reais.

Os seis valores públicos de preparação abaixo já foram configurados no projeto, no ambiente Production:

| Variável | Valor |
| --- | --- |
| `PORTAL_PUBLIC_URL` | `https://remotecontroller-portal.vercel.app` |
| `PORTAL_ALLOW_HTTP_LOOPBACK` | `false` |
| `PORTAL_PRICE_CENTS` | `990` |
| `SALES_ENABLED` | `false` |
| `ASAAS_ENV` | `sandbox` |
| `REMOTE_SERVICE_AVAILABLE` | `false` |

Para cadastro e login, faltam as variáveis do **projeto Supabase do portal**:

| Variável | Onde obter / tratamento |
| --- | --- |
| `NEXT_PUBLIC_SUPABASE_URL` | URL do projeto Supabase |
| `NEXT_PUBLIC_SUPABASE_ANON_KEY` | Chave anon/publicável do projeto |
| `SUPABASE_SERVICE_ROLE_KEY` | Chave de serviço; marcar como Secret/Sensitive; somente servidor |

Para testar cobrança e confirmação pelo Asaas, faltam:

| Variável | Onde obter / tratamento |
| --- | --- |
| `ASAAS_API_KEY` | Chave da conta **sandbox** Asaas; Secret/Sensitive |
| `ASAAS_WEBHOOK_TOKEN` | Segredo aleatório independente de pelo menos 32 caracteres; Secret/Sensitive; usar o mesmo no webhook Asaas |

Não enviar esses valores pelo chat, não colocar em `vercel.json`, não usar `NEXT_PUBLIC_` em chaves privilegiadas, não passar segredos por `--value`/`--env` na linha de comando e não versionar arquivos `.env`.

A opção `vercel env ls --json` da CLI atual inclui valores legíveis; não imprimir/compartilhar essa saída. Para consultar os nomes, use a listagem padrão, sem JSON ou debug.

Ainda no Supabase:

1. Aplicar `supabase/migrations/202610080001_commercial.sql` no banco do portal.
2. Habilitar confirmação de e-mail e configurar SMTP apropriado para usuários externos.
3. Definir Site URL `https://remotecontroller-portal.vercel.app` e permitir o callback exato `https://remotecontroller-portal.vercel.app/auth/callback`.

## Comandos para continuar depois da configuração

Abra PowerShell neste diretório (no checkout da branch `feat/commercial-portal`):

```powershell
Set-Location -LiteralPath 'C:\Users\Notebook\.codex\worktrees\commercial-portal\Remote Desktop Controller\apps\portal'
$env:NO_UPDATE_NOTIFIER = '1'
vercel whoami
vercel env ls production --scope andrefonns-projects --project remotecontroller-portal
```

Se a autenticação expirar, execute `vercel login` e autorize **a conta existente** no navegador. Não criar outra conta e não importar o repositório pelo painel Git.

O vínculo local já existe neste computador. Em outro checkout/máquina, vincule o projeto **existente**:

```powershell
vercel link --yes --scope andrefonns-projects --project remotecontroller-portal
```

Não usar `--repo`, `vercel git connect` ou integração GitHub. Não reutilizar o projeto `toemdia`. Se precisar criar o projeto em outra conta autorizada, use `vercel project create` separadamente e depois vincule o projeto existente, em vez de aceitar uma importação Git automática.

Valide localmente antes do envio:

```powershell
npm ci
npm test
npm run lint
npm run build
vercel deploy --dry --json --scope andrefonns-projects --project remotecontroller-portal
```

O dry-run apenas inspeciona o pacote de upload; não cria deployment. `.vercelignore` exclui `.env*`, `.vercel`, `.git`, `node_modules`, `.next`, testes, migrations e artefatos locais. Migrations permanecem no repositório e devem ser aplicadas separadamente no Supabase.

Após configurar as variáveis necessárias, publique **somente esse projeto novo**:

```powershell
vercel deploy --prod --yes --scope andrefonns-projects --project remotecontroller-portal
```

O uso de `--prod` escolhe o ambiente Production **do projeto `remotecontroller-portal`** e seu endereço estável `vercel.app`, garantindo que `PORTAL_PUBLIC_URL` e a origem do navegador coincidam. Não faz merge, não muda a branch main, não altera o projeto do blog e não ativa pagamentos reais. `ASAAS_ENV=sandbox` e `SALES_ENABLED=false` continuam controlando a operação comercial.

Não usar `--public` (expõe código-fonte), não adicionar `toemdia.blog.br` a esse projeto e não executar comandos de DNS/domínios/alias para o blog.

## Conferir o deployment e o webhook

1. Aguarde o build remoto terminar e confirme que a URL retornada abre a landing, `/login` e `/register`.
2. Confira `/api/config`: deve mostrar `sandbox=true`, `salesEnabled=false` e `serviceAvailable=false`; nunca deve incluir chaves ou tokens.
3. Confira se a proteção de deployment da Vercel está bloqueando usuários externos ou o Asaas. Ajustar, quando necessário, somente a proteção do novo portal; não alterar o blog.
4. No sandbox Asaas, cadastrar POST `https://remotecontroller-portal.vercel.app/api/asaas/webhook`, com token de autenticação igual a `ASAAS_WEBHOOK_TOKEN`. Selecionar eventos de cobrança/recebimento/atualização/cancelamento/estorno tratados pela aplicação.
5. Para exercitar checkout de teste, habilitar `SALES_ENABLED=true` **mantendo `ASAAS_ENV=sandbox`**, redeploy e verificar um pagamento sandbox real, webhook e crédito sem data de ativação. Testes locais não substituem essa prova externa.

Antes de dinheiro real: configurar prazo máximo de entrega, operador, suporte e política de reembolso (`DELIVERY_DEADLINE`, `LEGAL_OPERATOR_NAME`, `SUPPORT_EMAIL`, `REFUND_POLICY`), revisar a oferta e somente depois trocar as credenciais/ambiente Asaas e abrir vendas. A VPS e o gate comercial remoto continuam fora desta publicação inicial.

## Evidência desta preparação

- `npm test`: 49 testes passaram em 5 arquivos.
- `npm run lint`: zero erros; três advertências existentes.
- `npm run build`: compilação, TypeScript e geração das 27 páginas/rotas concluídos com exit code 0.
- CLI autenticada no escopo pessoal; projeto Next criado sem conexão Git, comprovado por consulta à API.
- Dry-run final: 53 arquivos (incluindo este guia), `package-lock.json` e `vercel.json` presentes, diretório de upload `apps/portal`, nenhum caminho `.env*`, `.vercel`, `.git`, `node_modules`, `.next`, testes ou migrations incluído.
- A publicação foi interrompida na etapa de variáveis de ambiente; não houve deploy público ou alteração do domínio atual.

Referências oficiais: [deploy CLI](https://vercel.com/docs/cli/deploy), [vínculo local](https://vercel.com/docs/cli/link), [escopo](https://vercel.com/docs/cli/global-options).

## Atualização integrada de 2026-10-09

A autorização passou a reconciliar o pagamento canônico e estornos DONE antes do uso, independentemente da disponibilidade do evento parcial no painel. Aplicar a nova migration `202610090001_verified_authorization.sql` antes de habilitar MCP. Portal publicado com vendas e serviço indisponíveis permanece seguro durante essa etapa. Evidências e requisitos de migração dos usuários legados: `docs/commercial-portal/final-validation.md`.
