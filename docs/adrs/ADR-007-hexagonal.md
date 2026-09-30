# ADR-007 — Arquitetura hexagonal
Status: aceito. Seis crates de biblioteca e dois apps. Domain é puro; ports
pequenos; application conhece contratos; adapters tecnológicos ficam coesos em
infrastructure. Transport é separado para preservar a fronteira do gateway.
Arc dyn Tool é usado para extensão; repos conhecidos têm generic dispatch.
