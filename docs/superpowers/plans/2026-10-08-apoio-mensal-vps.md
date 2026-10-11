# Plano de apoio mensal para manter a VPS

Data: 8 de outubro de 2026.

**Atualização aprovada:** o responsável escolheu acesso pago. Pagamentos anteriores à ativação garantem 30 dias completos a partir do primeiro login de uso com o serviço disponível. Isso substitui as hipóteses de apoio voluntário e período de calendário deste estudo. A implementação e o guia atual estão em `C:/Users/Notebook/.codex/worktrees/remote-product/Remote Desktop Controller/docs/product/paid-access.md` e `paid-access-operation.md` no mesmo diretório.

## Objetivo e premissas

Receber apoio mensal de R$ 9,90 da comunidade, acompanhar a arrecadação líquida e usar o saldo para manter a VPS e sua recuperação. Este é um plano operacional e de arquitetura; não implementa cobrança ou modifica o serviço.

Premissa proposta, ainda sujeita à decisão do responsável: apoio voluntário. O apoio mantém a infraestrutura compartilhada; não concede automaticamente um direito exclusivo de acesso. Se o pagamento for condição para usar o serviço hospedado, aplicar a variante de acesso pago descrita abaixo.

Link informado: https://www.asaas.com/000/c/4kcm8rtyfv8lxck2 . A ferramenta de consulta não conseguiu abrir essa página. Valor, recorrência, modalidades e notificações precisam ser verificados dentro da conta Asaas. O endereço público não comprova essas configurações.

## Alternativas

| Alternativa | O que entrega | O que exige |
| --- | --- | --- |
| Link Asaas e acompanhamento no painel | Começar a receber apoio recorrente | Configuração da conta e rotina financeira; sem backend próprio |
| Link Asaas e integração de acompanhamento | Página de meta, status de apoiador e relatório automático | HTTPS, API, webhook, banco e associação segura de identidade |
| Assinatura individual criada pelo servidor | Cobrança vinculada diretamente à conta e possível acesso pago | Login, API, estados de criação, webhook e controle de acesso |

Recomendação: começar com a primeira alternativa, preparar a segunda quando houver necessidade real de acompanhamento no produto, e usar a terceira se a mensalidade for obrigatória para acesso.

## Etapa 1 — preparar a conta e o link

- [ ] Confirmar cadastro aprovado, titular responsável, acesso ao painel e conta de destino para os valores recebidos.
- [ ] Conferir o link: nome “Apoio mensal à VPS”, valor R$ 9,90, tipo assinatura/recorrente e periodicidade mensal. Na API de links, a configuração equivalente é `chargeType=RECURRENT` e `subscriptionCycle=MONTHLY`.
- [ ] Confirmar Pix entre as modalidades disponíveis; não confundir assinatura Pix com autorização de Pix Automático.
- [ ] Definir primeiro vencimento, notificações e canal de contato para cancelamento.
- [ ] Para apoio voluntário, configurar a campanha sem multa, juros ou cobrança coercitiva; revisar o comportamento das cobranças vencidas e seu cancelamento no painel.
- [ ] Conferir tarifas reais da conta, inclusive serviços de notificação e transferências, antes de calcular a meta.
- [ ] Definir tratamento dos recebimentos e documentos com o responsável contábil; chamar de apoio não determina por si só o tratamento fiscal.

## Onde e como a comunidade paga

1. O apoiador encontra o botão “Apoiar com R$ 9,90 por mês” na comunidade ou em uma página pública de apoio.
2. O botão abre o link hospedado pelo Asaas.
3. O apoiador preenche os dados exigidos pelo Asaas e recebe a primeira cobrança.
4. Na modalidade Pix comum, paga pelo aplicativo do banco usando o QR Code ou Pix Copia e Cola.
5. O pagamento é acompanhado no painel Asaas. Na etapa automatizada, o webhook registra o recebimento.
6. A assinatura gera as cobranças dos próximos ciclos; com notificações configuradas, o apoiador recebe o link da cobrança correspondente e paga novamente.

Não pedir ao apoiador que refaça o cadastro no link geral a cada mês: isso pode criar novas assinaturas e clientes duplicados. Para renovar, usar a cobrança da assinatura existente.

Pix comum exige uma ação do apoiador a cada cobrança. Pix Automático exige uma autorização específica e integração/configuração própria. Se for adotado futuramente, testar autorização ativa, débito, falta de saldo, cancelamento e recuperação; não inferir sua ativação pelo primeiro Pix pago.

O dinheiro vai para a conta Asaas do responsável, conforme a liquidação. O responsável paga a VPS ou transfere para a conta usada no provedor. O link não paga a VPS automaticamente.

## Etapa 2 — operação financeira e transparência

- [ ] Registrar custo total mensal em reais: VPS, impostos e câmbio aplicáveis, domínio dividido por 12, backup externo e custos efetivos de recebimento.
- [ ] Definir uma reserva inicial de um mês de infraestrutura; a arrecadação excedente alimenta a reserva.
- [ ] Calcular o número necessário de apoiadores pelo valor líquido, não pelo valor bruto anunciado.
- [ ] Conferir semanalmente cobranças recebidas, tarifas, saldo disponível e vencimento da VPS.
- [ ] Pagar a VPS antes do vencimento e guardar o registro financeiro em local privado.
- [ ] Publicar mensalmente totais agregados: recebido bruto, tarifas, devoluções, líquido, gastos e reserva. Não publicar CPF, e-mail ou nomes sem autorização.
- [ ] Medir o total realmente recebido no mês; assinaturas criadas e cobranças pendentes não contam como dinheiro arrecadado.
- [ ] Se a meta não for atingida, usar a reserva e informar a situação à comunidade. Decidir antecipadamente se o responsável cobre a diferença ou reduz a infraestrutura.

Fórmula: `apoiadores = arredondar_para_cima(meta_mensal / (9,90 - tarifa_por_recebimento))`. Se houver tarifa percentual ou custos variáveis, incluir no cálculo do líquido.

Exemplo ilustrativo com tarifa de R$ 1,99: líquido de R$ 7,91 por apoio. Metas totais de R$ 60, R$ 80 e R$ 100 exigem respectivamente 8, 11 e 13 apoiadores. Esses valores não representam o custo atual da VPS nem a tarifa contratada da conta. Reserva adicional e falta de pagamentos aumentam a meta.

## Etapa 3 — acompanhamento automático, quando necessário

### Componentes

- Página pública de apoio: finalidade, valor, recorrência, botão Asaas, meta líquida, contato e instruções de cancelamento.
- Área do apoiador, se houver login: assinatura vinculada, período de apoio, histórico e cobrança atual.
- Backend HTTPS: integração com API Asaas, endpoint de webhook e reconciliação periódica.
- Banco de dados: apoiadores, assinaturas, cobranças, eventos recebidos, ações administrativas e agregados financeiros.
- Visão administrativa: recebimentos, pendências, cancelamentos, eventos com erro e divergências.

### Associação entre pagamento e conta

O link geral não identifica sozinho qual conta interna deve ganhar um benefício. O Asaas pode criar um novo cliente em cada uso do link; não presumir unicidade por e-mail ou CPF.

Para apoio sem benefícios individuais, acompanhar pelo painel e pelos IDs Asaas basta. Para status individual, vincular uma assinatura a uma conta autenticada com verificação de titularidade; não aceitar apenas o ID digitado ou um comprovante.

Para acesso pago, preferir criar/reutilizar o cliente e criar a assinatura pelo backend após login, usando uma referência interna opaca e persistindo os IDs retornados. Encaminhar a pessoa à página da cobrança individual hospedada pelo Asaas. Não colocar segredos ou dados pessoais na URL. Reservar a criação no banco antes da chamada externa e reconciliar respostas incertas, evitando assinaturas duplicadas por clique ou timeout.

### Dados mínimos

- Apoiador: ID interno, conta interna opcional, contato necessário e consentimento separado para divulgação pública.
- Assinatura: ID Asaas, cliente Asaas, campanha de origem, valor em centavos, ciclo, estado e vínculo verificado.
- Cobrança: ID Asaas único, assinatura, modalidade, valor bruto/líquido, vencimento, recebimento, devolução e período de apoio.
- Evento: ID único, data, tipo, cobrança, resultado de processamento e tentativas.

Guardar apenas dados necessários; não coletar credenciais bancárias ou cartão na aplicação. Se o sistema operar por IDs do Asaas, não copiar CPF para a base local sem necessidade.

### Processamento seguro

1. Configurar uma chave de API no servidor e um segredo independente para autenticar o webhook pelo header `asaas-access-token`.
2. Validar autenticação, tamanho e formato do evento; consultar a API para verificar a cobrança antes de atribuir benefício.
3. Conferir campanha/assinatura, cliente, valor de 990 centavos, modalidade e estado. A integração pode receber pagamentos de outras campanhas da mesma conta.
4. Registrar cada evento de forma durável e processar com deduplicação por ID de evento e ID de cobrança.
5. No Pix, usar o recebimento confirmado (`PAYMENT_RECEIVED`). Criar assinatura, gerar QR Code e retornar ao site não comprovam pagamento.
6. Atualizar cobrança e status do apoiador em transação local. Não manter transação do banco aberta durante consulta ao Asaas.
7. Tratar notificações repetidas e fora de ordem sem regredir um estado recebido para pendente. Em dúvida, consultar o estado atual na API.
8. Tratar vencimento, cancelamento e estorno preservando o histórico; cancelar a assinatura não apaga um período já pago.
9. Conciliar diariamente cobranças recentes e pendências com a API paginada; alertar em falha do webhook ou divergência persistente.
10. Manter segredos fora de logs, repositório, frontend e instalador do agente; restringir acesso administrativo.

Ao restaurar backup antigo, manter atribuições de benefícios suspensas até reconciliar pagamentos, cancelamentos e revogações com o estado atual do provedor.

## Apoio voluntário e variante de acesso pago

No apoio voluntário, faltar um pagamento altera somente o status de apoiador do período, não bloqueia automaticamente o serviço. Oferecer cancelamento fácil da recorrência e interromper novas cobranças; verificar separadamente cobranças já emitidas.

Se o responsável escolher acesso pago, apresentar claramente essa condição antes da contratação. Criar direito de uso do serviço hospedado por período mensal de calendário, com política explícita para finais de mês, antecipação e atraso. Cada cobrança paga cobre um período definido uma única vez; estorno recalcula o direito considerando outros períodos válidos. Cobrança futura vencida não deve invalidar tempo ainda pago.

A validação deve ocorrer no servidor que atende o serviço remoto. Ao expirar, manter login, histórico, renovação e cancelamento disponíveis e rejeitar as operações protegidas. O runtime local deve continuar segundo sua licença e suas regras próprias; a mensalidade da VPS não deve ser aplicada ao daemon local inadvertidamente.

## Recuperação e disponibilidade

Manter exportação periódica dos relatórios financeiros e backup externo criptografado dos registros locais. Fazer snapshot consistente de SQLite com a API de backup ou `VACUUM INTO`, verificar integridade e testar restauração isolada. Guardar acesso administrativo ao Asaas e ao provedor fora da VPS: uma queda do servidor não deve impedir consultar arrecadação ou pagar a infraestrutura.

## Testes e critérios de entrada em operação

- Link confirmado como recorrente mensal de R$ 9,90 e modalidade Pix disponível.
- Primeiro apoio concluído e identificado no painel; próxima cobrança pertence à mesma assinatura.
- Processo de cancelamento testado, inclusive destino das cobranças já emitidas.
- Tarifa e líquido conferidos no extrato da conta.
- Na integração: pagamento de teste vinculado à pessoa correta; outro usuário não pode reivindicá-lo.
- Evento repetido não aumenta a arrecadação nem concede período adicional.
- Webhook inválido e cobrança de outra campanha não alteram status.
- Falha temporária e evento fora de ordem recuperados por reconciliação.
- Estorno tratado sem apagar histórico ou ignorar períodos válidos.
- Se acesso pago: teste de expiração e de renovação diretamente no servidor.
- Após sandbox, realizar um pagamento real autorizado para verificar o fluxo de produção e conferir sua tarifa; nenhum pagamento foi realizado durante este planejamento.
- Backup restaurado em isolamento e registros conciliados antes de voltar à operação.

## Ordem de execução e responsáveis

1. Responsável pela campanha: decidir apoio voluntário ou acesso pago, confirmar configurações do link, tarifa, custo mensal total e contato de cancelamento.
2. Responsável pela campanha: publicar a explicação e o link na comunidade e iniciar acompanhamento no painel.
3. Responsável financeiro: executar pagamento da VPS e prestação de contas mensal.
4. Desenvolvimento, se necessário: criar acompanhamento automático em serviço hospedado apropriado, com identidade verificada, API, webhook, reconciliação e testes.
5. Operação: validar em produção, monitorar falhas e exercitar restauração.

Não contratar uma segunda VPS apenas para cobrar o apoio. Usar primeiro o painel Asaas; se houver backend próprio, integrar ao serviço hospedado existente após confirmar sua localização e capacidade.

## Informações ainda necessárias

- Apoio voluntário ou requisito para acesso hospedado.
- Custo total mensal real da infraestrutura, em reais.
- Configuração efetiva do link, tarifa contratada e notificações habilitadas.
- Comunidade/canal onde o botão será publicado e contato responsável.
- Se houver integração, domínio HTTPS, serviço hospedado/checkout correto, login existente e forma segura de disponibilizar segredos ao servidor.

Neste checkout, o README e os diretórios verificados descrevem o runtime local. A integração Asaas não foi encontrada em apps, crates, migrations ou docs. Há histórico de trabalho remoto em outro checkout; confirmar esse alvo antes de planejar alterações em arquivos de aplicação.

## Fontes consultadas em 8 de outubro de 2026

- Criando um link: https://docs.asaas.com/docs/criando-um-link-de-pagamentos
- Configuração pelo painel: https://central.ajuda.asaas.com/hc/pt-br/articles/32053358645787-Como-criar-um-link-de-pagamento-no-Asaas
- Assinaturas: https://docs.asaas.com/docs/assinaturas
- Pix comum e Pix Automático: https://docs.asaas.com/docs/diferen%C3%A7a-entre-pix-autom%C3%A1tico-e-assinaturas-1
- Eventos de cobrança: https://docs.asaas.com/docs/webhook-para-cobrancas
- Autenticação de webhook: https://docs.asaas.com/docs/about-webhooks
- Tarifas públicas, sujeitas ao contrato: https://www.asaas.com/precos-e-taxas
