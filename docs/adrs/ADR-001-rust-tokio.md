# ADR-001 — Rust e Tokio
Status: aceito. Rust protege ownership e estados; Tokio multiplexa IPC,
processos e cancelamento. Domain e compilação pura permanecem síncronos.
Trabalho de C/CPU/disco bloqueante usa workers limitados. Alternativa rejeitada:
uma thread por operação ou async decorativo em entidades puras.
