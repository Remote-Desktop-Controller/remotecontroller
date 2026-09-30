# Executar o runtime Windows

Extraia todos os arquivos do ZIP. Os dois executáveis funcionam sem Node,
Python, Docker ou instalação de Rust. Não iniciar o gateway sozinho antes do daemon.

1. Abra um terminal na pasta extraída e inicie o daemon:

```powershell
.\local-daemon.exe --workspace "C:\seu-projeto" --state-dir "$env:LOCALAPPDATA\LocalRuntime\seu-projeto" --endpoint "\\.\pipe\local-runtime" --allow-write
```

2. Configure seu host MCP para iniciar o gateway:

```json
{
  "mcpServers": {
    "local-runtime": {
      "command": "C:/PASTA_EXTRAIDA/mcp-gateway.exe",
      "args": [
        "--endpoint", "\\\\.\\pipe\\local-runtime",
        "--token-file", "C:/Users/SEU_USUARIO/AppData/Local/LocalRuntime/seu-projeto/ipc.secret"
      ]
    }
  }
}
```

A sintaxe de configuração do host pode variar. Substitua os paths de exemplo.
O state-dir deve ficar fora do workspace. Sem `--allow-write`, o default é leitura.
Processos estão desabilitados; só conceder os grants explícitos e regras exatas
após ler docs/security.md. Ctrl+C solicita encerramento e cleanup.

Esta entrega foi validada no Windows. Não há instalador, atualização automática
ou sandbox completa de processos. Os detalhes de testes, benchmarks e limites
estão em docs/validation.md e docs/benchmarking.md. SHA256SUMS.txt identifica
os executáveis do pacote. Código e documentos completos estão no repositório.
