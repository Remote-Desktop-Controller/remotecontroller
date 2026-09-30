# ADR-005 — Moka somente como cache
Status: aceito. Contexto compilado usa capacidade finita e TTL. Chave inclui
graph_version e budget; SSOT é consultado antes/depois. Cache vazio no restart
é normal. Não armazenar operações/checkpoints somente em RAM nem fazer write-back.
