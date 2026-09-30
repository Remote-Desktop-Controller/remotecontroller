# A — Integridade de arquivos

Ler docs/readiness/spec.md e este brief. Ownership exclusivo: domain/FileSnapshot
e novos VOs de metadata; files.rs; testes de arquivos/integridade; storage.rs
somente SnapshotDto/save_checkpoint/checkpoint decoding. Outros trechos pertencem
ao root. Não executar Cargo builds amplos ou commit; avisar quando pronto para gate.

Implementar preservação real de permissions/mode, timestamps e atributos/ACL
suportados via APIs nativas, com regressões Windows e Unix. Metadados devem viajar
nos snapshots (domínio puro, DTO serde somente infrastructure) e snapshots antigos
sem metadata devem continuar legíveis. Não devolver sucesso se uma propriedade
prometida não foi restaurada. Sem cópia do workspace inteiro.

Detectar writers externos: hashes/preconditions na preparação/aplicação e
rollback. Evitar sobrescrever silenciosamente edição externa; relatório deve
explicar conflito e manter journal para decisão/recovery segura. Diretórios por
handle cap-std, nunca canonicalize+std write por path ambiente. Preservar
proteções gitignore/excluídos/links/case variants e cancelamento/atomicidade.

Integrar com tools.rs por interface pequena: root fará mudanças necessárias
em Services::snapshot/apply_bytes; comunicar assinaturas e CamposFileSnapshot.
Usar defaults/constructores que facilitem ajustar os demais testes pelo root.

Teste antes do fix: executable bit/readonly/timestamps, binary rollback,
conflito de writer externo e atomic write root/nested. Não substituir os testes
por mocks de filesystem. Escrever report docs/readiness/report-files.md com APIs,
evidência, limites exatos e arquivos alterados. Não inventar garantia de ACL.
