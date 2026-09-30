# ADR-003 — Named Pipes e Unix Domain Sockets
Status: aceito. Transportes locais, sem porta pública, com proteção do usuário.
Frames u32 + MessagePack, negociação v1 e segredo local. Uma conexão por chamada
permite cancelamento independente e retry com IDs persistidos. Alternativa TCP
rejeitada por ampliar superfície de exposição sem necessidade.
