# Objetivo original e critérios de conclusão do produto

O objetivo do usuário, desde o início, é construir um produto equivalente ao
Remote Desktop Commander, com mais capacidade e confiabilidade. O runtime Rust
é um componente desse mesmo produto. O fechamento local da v0.2 não representa
o fechamento do produto completo; essa distinção corrige a interpretação anterior
do escopo, sem alterar o objetivo do usuário.

## Experiência obrigatória

O usuário instala o agente, entra na conta, conecta seu computador e adiciona o
plugin no ChatGPT. Depois trabalha nos arquivos e no terminal do computador pelo
site ou celular. Não deve precisar de chave de API de modelo, túnel particular,
arquivos de configuração MCP ou modo desenvolvedor no fluxo do produto publicado.
O suporte deve buscar a mesma disponibilidade do produto de referência, incluindo
Free, Plus e Pro, respeitando as regras de distribuição da plataforma.

## Componentes do mesmo produto

- Runtime local: executa ferramentas, aplica permissões, preserva histórico,
  checkpoints, operações e logs. A candidata v0.2 está implementada e validada.
- Serviço remoto MCP HTTPS: recebe chamadas do ChatGPT e encaminha ao dispositivo
  autorizado. Ainda não implementado.
- Contas e OAuth: login, autorização do cliente MCP, sessões e revogação. Ainda
  não implementados.
- Pareamento e conexão de dispositivos: autorização por código, conexão de saída
  do agente, reconexão, estado online e seleção de computador. Ainda não implementados.
- Instalação e gerenciamento: agente pronto para uso, execução persistente e
  gerenciamento claro de dispositivos e permissões. Instalação local existe;
  experiência remota ainda não implementada.
- Plugin publicado: metadados, documentação, políticas e submissão ao catálogo
  do ChatGPT. Ainda não preparado ou publicado.

## Capacidade e confiabilidade verificáveis

Cobrir arquivos, busca, criação de pastas, movimentação/renomeação, sessões de
terminal interativas, entrada/saída, processos e múltiplos computadores. A matriz
de equivalência deve distinguir funções completas, parciais e ausentes no código.

Demonstrar melhorias com testes de reconexão, cancelamento, repetição de chamadas,
recuperação após crash, retenção de saída, conflitos de arquivo e limites de
recursos. Regressões devem ter reprodução e evidência da correção. Não prometer
ausência absoluta de bugs ou superioridade sem comparação observada.

## Gate de entrega

Só declarar o produto pronto após um usuário conseguir entrar, parear um PC,
conectar pelo ChatGPT no navegador e realizar leitura, escrita e execução
autorizadas, com isolamento entre contas/dispositivos, revogação e reconexão
verificados. Build/CI do runtime local comprovam apenas esse componente.

As especificações de readiness v0.2 são registros do marco local. Este documento
define o escopo do produto que continua em construção.
