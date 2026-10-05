# Correção OAuth RFC 9207 — 2026-10-05

O deployment público responde health/discovery em HTTPS. A investigação confirmou issuer consistente nos dois documentos, mas suporte à identificação de issuer ausente; o código de consentimento também não incluía iss. Com a allowlist vazia relatada pelo proprietário, o DCR recusa o callback. São requisitos separados: implementar RFC 9207 e configurar a URI autorizada.

## Mudança

- Metadata anuncia authorization_response_iss_parameter_supported=true.
- Consentimento adiciona iss de state.config.public_url antes de ramificar sucesso/recusa; portanto ambos os redirects ao cliente têm o mesmo issuer exato.
- Os outros redirects são internos de login/logout/gestão, não Authorization Responses. Erros de entrada/CSRF/resource/redirect inválidos continuam HTTP locais e não redirecionam para destinos não validados.
- DCR mantém correspondência exata e explícita; nenhum wildcard/CIMD/fetch de URL foi acrescentado. Lista vazia continua recusada.
- deployment/.env.example e manual configuram o callback estável para ChatGPT/Codex elegíveis.

Referências: [OpenAI Authentication](https://developers.openai.com/plugins/build/auth) e [RFC 9207](https://www.rfc-editor.org/rfc/rfc9207.html).

## Regressões

Três testes falharam antes do patch: falta da flag e falta de iss em sucesso/negação. Após o patch, os 12 fluxos e dois unitários do serviço passaram. A suíte inclui metadata/issuer/authorization_servers, callback estável com allowlist explícita, recusa de slash/query/host/URI diferentes e lista vazia, sucesso/recusa com iss único e state UTF8, query pré-existente preservada, PKCE S256 e resource/redirect inválidos sem redirect ao cliente. Teste de token também recusa redirect divergente antes do consumo do código.

Gates globais aprovados: cargo fmt --check, build do workspace, cargo test --workspace (121 testes) e Clippy --workspace --all-targets --all-features -D warnings. Flags --locked/--offline usaram dependências já disponíveis. Avisos de dependências vendorizadas permanecem; nenhum erro do Clippy do workspace. Logs locais ficam no worktree, fora do Git.

## Atualizar o servidor existente

No diretório do checkout já implantado, preserve deployment/.env e os volumes:

```sh
git fetch origin
git switch codex/remote-product
git pull --ff-only
```

Altere/adicione somente esta linha em deployment/.env, mantendo domínio/email/valores existentes:

```env
RDC_REDIRECT_URIS=https://chatgpt.com/connector_platform_oauth_redirect
```

Recompile e recrie o serviço com a configuração atualizada:

```sh
docker compose --env-file deployment/.env -f deployment/compose.yaml up -d --build --force-recreate service
docker compose --env-file deployment/.env -f deployment/compose.yaml logs --tail=50 service
```

Confira no discovery público a flag true e o issuer idêntico ao authorization_servers do protected resource. Recrie/refaça a conexão ChatGPT para reler a descoberta e concluir OAuth. Não é necessário recriar VPS, parar Caddy ou remover volumes. Um restart sem rebuild mantém o binário antigo.

Este patch não foi implantado automaticamente: o acesso/titularidade do servidor permanece com o proprietário. A criação da conexão no ChatGPT só pode ser declarada resolvida após essa atualização e reteste real.
