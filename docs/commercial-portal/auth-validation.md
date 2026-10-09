# Correção de autenticação — 2026-10-09

## Resultado

Portal publicado em https://remotecontroller-portal.vercel.app, código `84fb32f`,
deployment READY `dpl_2CeCpACw48rvFimoEPmLWePvYN8z`. Commit principal `45e7543`.
Ambos enviados a feat/commercial-portal; main preservada.

O cabeçalho anterior sempre mostrava links de visitante. Agora layout, cabeçalho
e telas da conta usam uma sessão compartilhada obtida de um endpoint validado
com Supabase getUser e email_confirmed_at. O estado inicial é neutro, e erros de
verificação não são apresentados como logout. Nenhum token é disponibilizado
ao componente cliente.

Logout compartilha uma única promessa entre cliques concorrentes e remove os
dados da conta da tela imediatamente. Mudanças de identidade cancelam consultas
antigas; logout entre abas invalida a sessão e oculta dados anteriores. Logout
usa scope local, preservando outras sessões do usuário.

O dashboard exige identidade verificada no servidor. O proxy renova cookies e
nega visitantes com 307 antes do streaming; a página e as APIs mantêm suas
próprias validações. Falha temporária de Auth retorna 503. Cookies continuam
HttpOnly/Secure/SameSite=Lax e respostas de sessão não são armazenadas em cache.

Callback verifica novamente o usuário após PKCE/OTP, exige e-mail confirmado,
recusa parâmetros ambíguos e usa redirecionamento interno validado. Recovery OTP
não é tratado como cadastro, pois não existe interface de redefinição de senha.

## Evidência

- Suíte final: **13 arquivos, 112 testes aprovados, zero falhas/zero skips**,
  80,59 s. Incluiu a integração Rust/daemon existente, com serviços financeiros
  simulados e SQL isolado. Não houve cobrança real.
- Build webpack e typecheck separados aprovados. ESLint: zero erros, quatro
  advertências (exportação anônima PostCSS e navegação interna com recarga).
- Testes de regressão RED foram observados antes das correções do cabeçalho,
  callback, tratamento de indisponibilidade e redirecionamento anterior ao streaming.
- Revisão independente não encontrou bloqueios restantes. Dois problemas de
  concorrência identificados na revisão foram corrigidos e validados.
- Nove grupos de navegador local com fixtures passaram para carregamento neutro,
  autenticação, mobile, falhas/logout, resposta atrasada, invalidação entre abas,
  login/next, logout concorrente e ocultação de dados da conta. Essa execução
  precedeu o último ajuste de agendamento da consulta; a suíte completa e a
  validação real abaixo foram posteriores.

## Produção — HTTP público

| Requisição sem cookies | Resultado |
| --- | --- |
| GET / | 200 |
| GET /login | 200 |
| GET /dashboard | 307 para /login?next=%2Fdashboard; private, no-store |
| GET /api/auth/session | 200, user null; no-store |
| GET /api/account | 401 |
| Callback com código controlado inválido | 303 para login?error=confirmation |
| GET /api/config | salesEnabled false; serviceAvailable false |

## Produção — navegador com a conta real do proprietário

O usuário autenticou-se manualmente; nenhuma senha foi recebida no chat ou lida
pela automação. Antes da correção, a divergência foi reproduzida no dashboard.
Após a publicação, o cabeçalho exibiu Meu painel/Sair da conta, o dashboard
exibiu E-mail confirmado para a mesma identidade validada pelo servidor e a
sessão permaneceu correta após refresh.

Em viewport 390x844, o menu autenticado e a navegação para o painel passaram,
sem overflow horizontal. Logout real retornou à landing com Entrar/Criar conta.
Acesso posterior ao dashboard redirecionou ao login; refresh manteve o visitante
fora da área protegida. Nenhum erro JavaScript foi observado. O viewport foi
restaurado ao final. Capturas reais foram exibidas durante a validação.

Não foi emitido outro e-mail de confirmação nem criada uma conta de teste real.
Novos links de confirmação foram cobertos por testes controlados do callback;
a confirmação da conta já existente foi observada na sessão real.

Um servidor local de produção com variáveis fictícias foi bloqueado pela revisão
automática (apenas blocked by policy informado). Essa tentativa não iniciou o
servidor. A validação final foi concluída pelo caminho mais seguro: deployment
Vercel e navegador autenticado real, com autorização do usuário.

## Escopo preservado e auditoria SSH pendente

SALES_ENABLED=false foi confirmado também pela leitura do valor bruto da Vercel.
Não houve alteração de segredos, domínio, VPS ou regras financeiras. As mudanças
locais anteriores da migration e de sua auditoria foram preservadas fora dos
commits de autenticação; nenhuma migration foi executada nesta tarefa.

A auditoria SSH anterior continua pendente de execução pelo proprietário, pois
o acesso é por senha interativa e não há chave/agente local autorizado. O coletor
`vps-readonly-audit.ps1` prepara somente leituras filtradas; não foi executado
contra a VPS. A implantação, backup/rollback com caminhos reais e a migração dos
usuários antigos dependem desse inventário. Preencher as duas variáveis
REMOTE_SERVICE_COMMERCIAL_* e recriar o serviço já ativa o gate; não fazê-lo
antes da validação e autorização específica.

Referência SSR: https://supabase.com/docs/guides/auth/server-side/creating-a-client?queryGroups=framework&framework=nextjs
