# Tasks e status — produto completo

Atualizado: 2026-10-04. Fase: implementação. Base: ba5ee3a.

Status permitidos: concluída, em andamento, pendente, depende de ação humana, depende da plataforma. Cada conclusão exige evidência.

| ID | Task | Status | Evidência / critério |
|---|---|---|---|
| 00 | Runtime local v0.2 | Concluída no marco anterior | docs/readiness/progress.md; baseline desta evolução pendente |
| 01 | Contratos e plano de conclusão | Em andamento | docs/product/spec.md, plan.md |
| 02 | Serviço remoto, contas e OAuth | Em andamento | HTTP real, isolamento e fluxo PKCE |
| 03 | Sessões duráveis e contexto | Em andamento | migração, retomada, busca e isolamento |
| 04 | Agente remoto, pairing e reconexão | Pendente | daemon + agente + serviço reais |
| 05 | Ferramentas de arquivos e terminal | Pendente | stdin, PTY, operações de arquivo e recuperação |
| 06 | Onboarding e distribuição | Pendente | instalação/pacote/infra/plugin revisados |
| 07 | Revisão e validação final | Pendente | suíte, CI e cenários integrais |
| 08 | Ações manuais do proprietário | Depende de ação humana | último item; instruções concretas serão publicadas |

## Stats

- Baseline gerencial anterior: aproximadamente 30–40%; sem backlog ponderado histórico.
- Não há percentual novo comprovado nesta abertura. Não atribuir 99% antes dos gates.
- Subagentes previstos: dois implementadores por domínio; um revisor ao final, se necessário.
- Implementação local, validação hospedada e distribuição pública serão registradas separadamente.
- Ações humanas não incluem defeitos ou funcionalidades ainda não implementadas.
