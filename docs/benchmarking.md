# Benchmarks reproduzíveis

```text
cargo bench --no-run
cargo bench -p runtime-infrastructure --bench runtime
```

Criterion: 10 amostras, warmup 1 s, measurement 2 s por caso. Os números não são
garantias de SLA; buffers, disco, antivírus, caches do OS e hardware influenciam.
Executar sem compilação concorrente. Artefatos Criterion ficam em target/criterion.

Casos: cache_hit/cache_miss isolam Moka; event_insert inclui commit transacional
do evento/node/version; causal_query recupera até 128 eventos; file_scan_1000
enumera 1.000 arquivos respeitando política; batch_read_100 usa 16 leituras
concorrentes; context_compilation_128 mede compiler puro com budget 8192 bytes;
ipc_round_trip_with_handshake usa Named Pipe/UDS real, handshake e Heartbeat.

Dados são sintetizados exclusivamente em tempdirs. O dataset de causal_query
inclui eventos inseridos na etapa anterior; interpretar tamanho junto aos resultados.
Separar compiler puro do custo SQL de ContextEngine. Não extrapolar benchmark
de heartbeat para custo de uma escrita ou spawn. Resultados medidos ficam em
validation.md, preenchidos somente após execução bem-sucedida.

## Resultado observado no Windows

Intel Pentium 4417U @ 2.30 GHz, 2 cores / 4 threads, Rust 1.98.1 GNU, release.
Estimativas centrais Criterion; não são SLAs nem comparação válida com o primeiro
run interrompido. Log completo: evidence/criterion-final.log.

| Caso | Estimativa central |
|---|---:|
| Moka cache hit | 627.22 ns |
| Moka cache miss | 405.03 ns |
| Event insert + commit | 15.911 ms |
| Causal query, até 128 eventos | 5.5680 ms |
| Scan de 1.000 arquivos | 191.29 ms |
| Leitura de 100 arquivos, concorrência 16 | 17.435 ms |
| Context compiler, 128 eventos | 74.834 us |
| IPC round trip com handshake | 556.79 us |

Os oito casos terminaram com exit code 0 após substituir a consulta causal com
CTE por travessia limitada. O primeiro run com CTE foi preservado em criterion.log
para registrar o crash nativo, sem atribuir causa definitiva ao SDK.
