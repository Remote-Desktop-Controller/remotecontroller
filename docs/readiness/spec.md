# Entrega candidata completa — v0.2

Autorização: o usuário pediu concluir os pontos pendentes antes de seu primeiro
teste, incluindo segurança, integridade, durabilidade, instalação, contexto e
integração. Preservar núcleo existente e CI verde. Não reescrever arquitetura.

Camadas lógicas continuam domínio/aplicação/infraestrutura; dois binaries.
Gateway permanece adapter. Bootstrap/instalação ficam no composition root do
daemon, sem execução operacional dentro do gateway.

Critérios de aceitação:

1. Atomic writes e checkpoints preservam metadados/permissions suportados e
   detectam conflitos externos antes de substituir conteúdo ou restaurar snapshot.
2. Processos seguem policy local com programas/argv exatos, aprovação explícita,
   identity pinning e nenhum grant silencioso. Default nega spawn. Execução não
   confinada deve ser declarada/concedida; sem sandbox disponível, não alegar
   confinamento. Não permitir que host MCP aprove sua própria execução.
3. Stdout/stderr persistem progressivamente em spool privado, com limites de
   disco e indicador de truncamento; crash conserva saída capturada. Recovery
   reconcilia Starting/Running sem matar PID reutilizado. Cancelamento e encerramento
   de descendentes precisam de evidência em cada plataforma suportada.
4. Retenção/GC e backup do banco/checkpoints/logs possuem operações explícitas,
   configuráveis, conservadoras e verificadas em tempdirs, sem apagar trabalho
   do usuário. Migrations existentes devem preservar dados e rollback possível.
5. Contexto aceita task/intenção, produz rollups úteis e referências raw,
   respeita budget UTF-8 e graph version. Sem RAG/cloud obrigatório.
6. Instalação por usuário, conexão MCP com bootstrap automático do daemon,
   diagnóstico, atualização local verificada e desinstalação preservando dados.
   Usuário não precisa Node/Python/Docker/Rust para execução.
7. Integração real de host MCP, sem testes fake como substituto; configurações
   existentes preservadas. Host registration deve ser ação explícita de CLI.
8. Gates fmt/check/Clippy/test/bench e CI nos três OS; ZIPs/checksums, manual
   curto, RFC e matriz de evidências atualizados para candidato v0.2.

Assinatura pública exige certificado/identidade externa; não gerar confiança
fictícia ou comprar certificado. Se credencial não estiver disponível, entregar
artefato com checksum e identificar exatamente esse requisito externo.

Ruling: programas arbitrários não são suportados em modo seguro. Programas
confiáveis precisam de concessão administrativa explícita e regras exatas; a
interface deve explicar o acesso concedido. Proteção contra malware do mesmo
usuário não é prometida. Modelos cloud, painel e vetor ficam fora do escopo.
