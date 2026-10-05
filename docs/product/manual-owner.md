# Última tarefa: ações que exigem o proprietário

Este documento cobre somente acesso/titularidade de contas, domínio, HTTPS, callback real, identidade/políticas e a prova com sua conta ChatGPT. Gates, correções, pacote e validação automática são trabalho de engenharia; sua pendência não deve ser tratada como ação humana inevitável. Use os binários/ZIPs cuja evidência de release foi registrada, não um build antigo apenas porque existe.

## 1. Hospedagem e domínio reais

Escolha a conta/host autorizado e domínio sob sua titularidade. Aponte DNS para o host e permita portas 80/443. Prepare no diretório deployment um `.env` privado com `RDC_DOMAIN`, `RDC_TLS_EMAIL` e, no passo seguinte, `RDC_REDIRECT_URIS`. Não envie credenciais de hospedagem no chat ou no Git. O serviço interno 8787 permanece privado.

O repositório já prepara Dockerfile, compose e Caddy. No host provisionado, use `docker compose --env-file deployment/.env -f deployment/compose.yaml up -d --build`. Caddy emite HTTPS para o domínio e o volume service-data preserva banco de contas/relay. Autorize backup do volume e retenção real antes de abrir acesso público. Não apague esse volume ao atualizar.

Com hostname configurado, confira `https://SEU_DOMINIO/healthz`, `/.well-known/oauth-authorization-server` e `/.well-known/oauth-protected-resource/mcp`. O recurso deve ser exatamente `https://SEU_DOMINIO/mcp`; uma chamada sem token a /mcp deve exigir autenticação. Certificado válido e acesso externo são requisitos de deploy, não comprovação de uso no ChatGPT.

## 2. Conexão de teste e callback exato

Na conta ChatGPT que fará o teste, habilite Developer mode em Settings → Security and login, abra Plugins e adicione a URL HTTPS completa com `/mcp`. Acesso pode depender das permissões da conta/workspace; use o que a plataforma efetivamente disponibiliza. [Quickstart oficial](https://developers.openai.com/plugins/quickstart).

Copie o redirect URI exibido na página de gestão dessa conexão e registre o valor **exato** em `RDC_REDIRECT_URIS`. A implementação usa DCR e PKCE, sem CIMD; não escolha autenticação CIMD para este serviço. Não invente callback nem use wildcard. O callback pode ser específico da conexão; confirme a URL apresentada antes do teste e atualize o serviço com ela. [Autenticação oficial](https://developers.openai.com/plugins/build/auth).

O compose aceita callback vazio na primeira execução. Inicie o serviço HTTPS e crie a conexão de teste para obter o valor; a autorização OAuth é recusada enquanto o callback não entrar na allowlist. Atualize `.env` e recrie o serviço (`docker compose --env-file deployment/.env -f deployment/compose.yaml up -d --build`) antes de refazer a autorização. Complete login e consentimento na conta do produto; o host receberá access token por OAuth.

## 3. Parear e provar uso real

Instale o pacote no PC conforme INSTALL.md com ServerUrl real e workspace de teste. Aprove o código na mesma conta do produto que você autorizou no OAuth. Mantenha o agente ativo. Use uma pasta descartável para este teste e registre resultados observados sem tokens ou cookies:

1. No ChatGPT web, selecione o plugin, peça seus computadores e confirme nome/device_id correto.
2. Leia um arquivo conhecido; se habilitou escrita local, peça uma edição pequena e confira o arquivo no PC.
3. Crie sessão, grave uma decisão e retome o histórico em outra conversa usando session_id. Isso é continuidade operacional, não treinamento do modelo.
4. Desconecte/reconecte o PC. Retome uma operação com runtime_operation_id retornado e confirme que a mutação não se repetiu.
5. Se aprovou terminal localmente, abra/escreva/leia/feche um terminal de teste e confira fim do processo/filhos no PC.
6. Revogue a conexão OAuth na conta e confirme recusa de novas chamadas. Autorize novamente se necessário; revogue também o dispositivo e confirme que o agente perde autorização.
7. Repita device.list/leitura na interface móvel disponível para sua conta e anote o resultado real. Não extrapole o teste web para prometer suporte móvel comprovado.

Esse conjunto exige interação com a sua conta real. Se o host ou sua conta não disponibilizar a conexão/superfície necessária, registre a limitação da plataforma, sem marcar o teste como aprovado.

## 4. Identidade, políticas e publicação

Forneça nome real do publicador, site, suporte, privacidade e termos em HTTPS. Documente quais dados de conta/histórico/arquivos passam pelo serviço, retenção/revogação e o acesso confiável do terminal. Estes textos exigem suas decisões e identidade; os exemplos do repositório não substituem documentos públicos reais.

Use `tools/build-plugin.ps1` com esses valores e ServiceUrl real. O ZIP contém plugin.json, mcp.json, skill e casos de revisão; URL example.test é somente smoke e não pode ser enviada à revisão. Escolha a organização/projeto e identidade verificável no portal, faça upload do ZIP, resolva resultados da revisão e publique somente após aprovação. Upload não significa publicação. [Fluxo oficial de submissão](https://developers.openai.com/plugins/deploy/submission).

Se sua distribuição exigir assinatura pública Windows, autorize o certificado/identidade do editor e assine os binários antes de empacotar/verificar novamente os checksums. Não existe certificado obtido ou assinatura provada por este documento. Conserve versão anterior e o perfil do usuário nas atualizações.

Registre por fim hostname, checksum/versão efetivamente usados, data dos testes web/móvel, publicação/review ID e limitações ainda reais. Não atribua 99% ou produto público concluído a passos que não produziram evidência.

## Registrar a prova para a revisão

Quando a conexão real estiver funcionando, grave um vídeo curto mostrando login/consentimento, escolha do PC, leitura/edição de um arquivo descartável, retomada de sessão e revogação. Use uma conta/pasta de revisão separadas, com dados de teste e permissões mínimas. Hospede a gravação em URL acessível ao revisor e configure `extensions.com.openai.review.demo_recording_url` no pacote; os cinco casos positivos e cinco negativos já estão preparados. Não envie senha, cookies ou tokens no vídeo/ZIP. A [submissão oficial](https://developers.openai.com/plugins/deploy/submission) exige a demonstração para revisão de MCP.

Assinatura Windows, se exigida pela sua distribuição: use seu certificado real no computador de assinatura. Copie os quatro binários para uma pasta de saída separada, assine com a ferramenta do seu emissor ou `Set-AuthenticodeSignature` e confira `Get-AuthenticodeSignature` antes de gerar novamente o pacote. A documentação [Microsoft](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/set-authenticodesignature) descreve carregamento do certificado e SHA256; o timestamp desse cmdlet usa HTTP e não suporta HTTPS pela API subjacente, portanto siga o emissor ou use SignTool apropriado. Certificado/senha permanecem locais. Depois rode package-product.ps1 com BinaryDirectory apontando para os binários assinados e OutputDirectory novo; os hashes anteriores deixam de representar esses arquivos.
