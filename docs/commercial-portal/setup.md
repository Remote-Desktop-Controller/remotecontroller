# Publicar e configurar o portal comercial

O portal funciona independentemente do servidor MCP: Vercel hospeda login/compra/webhook; Supabase guarda a identidade e os direitos pagos; Asaas processa cobrança avulsa; VPS será conectada depois. Cada pagamento válido compra 30 dias. Não há renovação automática.

Domínio futuro: `toemdia.blog.br`. **Não alterar DNS, blog ou projeto Vercel do blog nesta etapa.** Criar um projeto independente e testar no endereço temporário `vercel.app`.

## 1. Supabase — projeto exclusivo do portal

1. Crie um projeto PostgreSQL/Supabase próprio para o portal. Não reutilize o banco de outro produto sem planejar isolamento.
2. Abra SQL Editor e aplique a migration em `apps/portal/supabase/migrations/202610080001_commercial.sql`. Alternativamente, vincule o projeto com Supabase CLI e execute as migrations via `supabase db push` a partir da pasta `apps/portal`.
3. Em configurações de API do projeto, obtenha URL, chave anon/publicável compatível e chave de serviço `service_role`. A chave de serviço só vai para variáveis do servidor; nunca é exposta no navegador.
4. Em Authentication, habilite e-mail/senha com confirmação de e-mail. Não desative confirmação para contornar falhas de entrega: a identidade comercial precisa ser verificável.
5. Configure SMTP próprio para enviar confirmação aos clientes. O SMTP padrão do Supabase é limitado e, sem SMTP configurado, só envia para membros autorizados da equipe do projeto. Um teste de signup local não comprova entrega de e-mail real.
6. Em URL Configuration, depois da primeira publicação defina Site URL como o endereço temporário exato do portal e permita `https://SEU-PROJETO.vercel.app/auth/callback`. Durante desenvolvimento, adicione explicitamente `http://localhost:3000/auth/callback`. Não usar wildcard genérico em produção.
7. Cadastre e confirme o usuário administrador e copie seu UUID para `ADMIN_USER_IDS`. Usuários não podem promover suas próprias contas pelo navegador.

## 2. Vercel — projeto novo, sem substituir o blog

1. Conecte o repositório GitHub `Remote-Desktop-Controller/remotecontroller`.
2. Crie um **novo projeto** com Root Directory `apps/portal`, framework Next.js, Node.js 22 ou superior.
3. Use a branch `feat/commercial-portal` para a implantação de teste. Não fazer merge em main só para publicar o portal. Configure a branch de produção deste projeto novo conforme o painel permitir, ou use um deployment preview de acesso público apropriado para o teste.
4. A Vercel executa `npm ci` e `npm run build` dentro de `apps/portal`; o lockfile do portal deve estar versionado. Rust não é compilado no deploy do portal.
5. Preencha as variáveis de `apps/portal/.env.example` em Settings → Environment Variables. Aplique aos ambientes utilizados e refaça o deploy após alterações.
6. Mantenha `SALES_ENABLED=false`, `ASAAS_ENV=sandbox`, `REMOTE_SERVICE_AVAILABLE=false`. A landing pode ser aberta sem credenciais; cadastro/compra dependem da configuração real do Supabase/Asaas.
7. Após obter o endereço temporário, defina `PORTAL_PUBLIC_URL=https://SEU-PROJETO.vercel.app`, atualize Supabase Site URL/callback e redeploy. Usar o endereço real fornecido pela Vercel, não literalmente o exemplo.
8. Verifique se Deployment Protection exige login Vercel no webhook. O Asaas precisa alcançar a rota pública; configure o deployment de teste para receber notificações sem login Vercel, mantendo a autenticação por token do endpoint. Não tratar uma página HTML de proteção como resposta do webhook.

## 3. Asaas — teste sandbox primeiro

1. No painel sandbox da sua conta Asaas, gere uma chave de API na área de integrações/API. Coloque-a diretamente no segredo `ASAAS_API_KEY` da Vercel. Não enviar pelo chat/Git.
2. Configure `ASAAS_ENV=sandbox`; a aplicação usa somente a origem oficial `https://api-sandbox.asaas.com/v3`. Produção usa `https://api.asaas.com/v3` quando selecionada explicitamente.
3. Defina `PORTAL_PRICE_CENTS` com o preço desejado: por exemplo, `990` representa R$ 9,90. O servidor salva esse valor no pedido; o navegador não escolhe o valor.
4. Gere um segredo longo aleatório para `ASAAS_WEBHOOK_TOKEN` (pelo menos 32 caracteres), diferente da chave de API e do segredo MCP.
5. Na área Webhooks do Asaas, crie endpoint POST com a URL:

```text
https://SEU-PROJETO.vercel.app/api/asaas/webhook
```

Essa URL passa a ser real **depois que o projeto estiver publicado**. Substitua SEU-PROJETO pelo nome real do deployment público e confirme acessibilidade. O domínio `toemdia.blog.br` não é necessário para esse teste.

6. No campo token de autenticação do webhook, use o mesmo valor de `ASAAS_WEBHOOK_TOKEN`; ele é enviado pelo Asaas no header `asaas-access-token`.
7. Selecione eventos de cobrança criada/atualizada/recebida/confirmada/vencida/deletada e eventos de estorno disponibilizados pelo painel. A aplicação reconsulta a API, verifica identidade/valor/modalidade/referência e processa de forma idempotente. Para Pix, `CONFIRMED` sob análise cautelar não libera crédito: aguarda `RECEIVED`.
8. Configure o retorno da cobrança individual para `/purchase?orderId=...` via aplicação. O retorno só mostra estado; não é comprovante de pagamento. O Asaas pode exigir que a URL de retorno pertença ao domínio cadastrado nas informações comerciais da conta. Ajuste isso no painel para o endereço real de teste, conforme as regras vigentes da conta.
9. O link geral antigo não é utilizado como checkout desse fluxo: cada usuário autenticado recebe uma cobrança **avulsa** associada ao pedido interno. Não criar assinatura mensal nem autorização Pix Automático para essa pré-venda.

## 4. Validar a pré-venda sem VPS

1. Mantenha o serviço remoto indisponível na configuração.
2. Confirme cadastro e login com e-mail real no Supabase.
3. Habilite apenas o teste sandbox de vendas conforme configuração da aplicação. Inicie checkout autenticado e pague usando recursos do sandbox Asaas.
4. Confira o evento recebido, pedido marcado pago e direito aguardando a disponibilidade. A tela deve exibir:

> Pagamento confirmado. Seu acesso de 30 dias ficará disponível quando o Remote Controller for liberado. O prazo começará no primeiro uso autorizado.

5. Entre novamente no portal, atualize a compra e confirme: o período não foi iniciado e nenhum dia foi consumido.
6. Repita a notificação, sincronize o pedido, teste pagamento pendente, cancelamento e reembolso. Nenhuma notificação repetida concede outro direito.
7. Confira isolamento com duas contas: uma não acessa pedidos/histórico/ativação da outra.

## 5. Antes de receber dinheiro real

O responsável deve configurar `DELIVERY_DEADLINE` (prazo máximo de entrega), `SUPPORT_EMAIL`, `LEGAL_OPERATOR_NAME` e `REFUND_POLICY` e revisar a oferta/termos apresentados. Não preencher esses campos com informações fictícias apenas para habilitar vendas.

Configure SMTP e as credenciais de produção Asaas, troque `ASAAS_ENV` para produção e somente então habilite vendas explicitamente. Confirme tarifas e regras reais na conta. Faça um pagamento real autorizado, verifique liquidação/webhook/histórico e confira o processo de reembolso. Não foi realizado pagamento real no desenvolvimento.

## 6. Quando a VPS estiver pronta

O banco operacional libSQL permanece na VPS. O Supabase comercial permanece independente: não copiar suas tabelas para substituir dispositivos/grants/relay.

1. Prepare a VPS/domínio MCP conforme documentação do produto existente.
2. No portal, configure `MCP_SERVICE_ORIGIN` com a origem HTTPS real do MCP e `MCP_SHARED_SECRET` com segredo aleatório de pelo menos 32 caracteres.
3. Na VPS, configure `REMOTE_SERVICE_COMMERCIAL_PORTAL_URL` com a origem HTTPS real do portal e `REMOTE_SERVICE_COMMERCIAL_SECRET` com o mesmo segredo. Habilitar só depois dos testes; se ambos estiverem ausentes, a configuração antiga permanece intacta.
4. Login OAuth na VPS passa pelo portal, que verifica Supabase e entrega ticket curto de uso único. Nonce, cookie, callback exato e troca servidor-servidor vinculam a conta ao grant. E-mail digitado ou token client-side não comprova vínculo comercial.
5. Valide pareamento/OAuth/metadados sem consumir dias. Mantenha `REMOTE_SERVICE_AVAILABLE=false` até a operação estar pronta.
6. Após prova pública do serviço, configure `REMOTE_SERVICE_AVAILABLE=true` no portal e redeploy. O primeiro comando remoto válido ativa um direito uma única vez, por exatamente 2.592.000 segundos. Login no portal/OAuth e criação de dispositivo não ativam.
7. Teste acesso direto à URL MCP, token anterior, retry de operação, nova conexão/dispositivo e expiração: nenhum pode contornar o gate. Portal indisponível impede novas operações comerciais.

## 7. Futuro domínio `toemdia.blog.br`

Somente depois da aprovação do teste e de um pedido explícito para alterar o domínio: adicionar o domínio ao projeto Vercel do portal, revisar os registros existentes, executar a mudança de DNS planejada e atualizar origem/callbacks/webhook. Não é parte dessa implementação e o blog será preservado.

## Fontes oficiais consultadas

- Supabase SSR: https://supabase.com/docs/guides/auth/server-side/nextjs
- Supabase SMTP: https://supabase.com/docs/guides/auth/auth-smtp
- Asaas avulso: https://docs.asaas.com/reference/criar-nova-cobranca
- Asaas eventos: https://docs.asaas.com/docs/webhook-para-cobrancas
- Asaas autenticação do webhook: https://docs.asaas.com/docs/about-webhooks
- Asaas retorno: https://docs.asaas.com/docs/redirecionamento-apos-o-pagamento
- Vercel monorepo: https://vercel.com/docs/monorepos
