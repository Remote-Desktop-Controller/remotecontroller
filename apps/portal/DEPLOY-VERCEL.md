# Deploy do portal pela Vercel CLI, sem integração Git

Preparação verificada em 09/10/2026. Branch de trabalho: `feat/commercial-portal`. Diretório de upload: **somente `apps/portal`**. O repositório privado continua na organização; não é necessário importar/conectar o GitHub ao projeto Vercel.

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
