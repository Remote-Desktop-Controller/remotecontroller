# RFC-0005 — Portal Comercial SaaS, Faturamento Integrado e Distribuição Tri-Plataforma v0.5.0

**Status:** Aprovado & Implementado  
**Versão Alvo:** v0.5.0  
**Data:** 2026-10-13  
**Autor:** Remote Desktop Controller Core Team  
**Repositório:** https://github.com/Remote-Desktop-Controller/remotecontroller  

---

## 1. Sumário Executivo

Para que o Remote Desktop Controller opere como um ecossistema sustentável para desenvolvedores individuais, equipes e empresas, o software requer uma interface central de integração, controle de acesso, licenciamento e distribuição confiável de binários.

O RFC-0005 especifica a arquitetura do **Portal Comercial SaaS (`apps/portal`)**, a integração com **Supabase (Autenticação e RLS)**, o gateway financeiro **Asaas (Assinaturas e Webhooks)**, a esteira de empacotamento unificada **v0.5.0**, e o centro de diagnóstico em tempo real.

---

## 2. Arquitetura do Portal SaaS

```mermaid
flowchart TD
    subgraph Cliente Web ["Portal Comercial (Next.js / React 19 / Tailwind)"]
        Onboarding[Onboarding & Pareamento Guiado]
        DiagCenter[Centro de Diagnóstico em Tempo Real]
        BillingView[Gerenciamento de Assinatura & Planos]
    end

    subgraph Backend & Dados ["Infraestrutura Cloud"]
        SupabaseAuth[Supabase Auth: JWT / OAuth]
        SupabaseDB[(Supabase PostgreSQL com RLS)]
        AsaasGW[Asaas Payment Gateway: PIX / Cartão]
        WebhookEndpoint["API Route: /api/webhooks/asaas"]
    end

    subgraph Dispositivos Conectados ["Desktop do Usuário"]
        DeviceAgent[Device Agent v0.5.0]
        LocalDaemon[Local Daemon v0.5.0]
    end

    Cliente Web --> SupabaseAuth
    Cliente Web --> SupabaseDB
    BillingView --> AsaasGW
    AsaasGW -->|Eventos de Pagamento| WebhookEndpoint
    WebhookEndpoint -->|Reconciliação de Plano| SupabaseDB
    DeviceAgent -->|Validação de Ticket / Cota| SupabaseDB
    DeviceAgent --> LocalDaemon
```

---

## 3. Modelo de Licenciamento: Cota Free Diária vs Plano Premium

O sistema adota um modelo Freemium transparente e idempotente:
1. **Tier Free:**
   - Permite vincular até 1 dispositivo.
   - Limite diário de operações remotas (renovado automaticamente a cada 24 horas).
   - Acesso completo ao runtime local, MCP Gateway e RDC Maestro.
2. **Tier Premium (Assinatura Recorrente):**
   - Dispositivos ilimitados pareados simultaneamente.
   - Execução ilimitada de ferramentas remotas.
   - Suporte prioritário e telemetria avançada de observabilidade.

### 3.1 Validação e Segurança de Webhooks do Asaas
A rota `/api/webhooks/asaas` processa eventos de ciclo de vida (`PAYMENT_CONFIRMED`, `PAYMENT_OVERDUE`, `SUBSCRIPTION_CANCELED`):
- Todas as requisições exigem token secreto no cabeçalho `asaas-access-token`.
- O payload é verificado contra replay attacks.
- As atualizações de status financeiro utilizam transações atômicas no PostgreSQL, garantindo que nenhum usuário receba direitos indevidos por eventos repetidos.

---

## 4. Centro de Diagnóstico em Tempo Real (`diagnostic-center`)

O portal inclui um painel de integridade de ponta a ponta que valida cada elo da cadeia de conexão:
- **Status do Daemon Local:** Verifica se o daemon está respondendo no IPC local.
- **Status do Device Agent:** Confirma se a conexão WebSocket com a nuvem está estabelecida.
- **Latência de Handshake:** Mede o tempo de resposta do handshake mútuo v2.
- **Status de Permissões:** Identifica se o modo `--full-computer` ou modo restrito de workspace está ativo.

---

## 5. Esteira de Distribuição e Empacotamento Tri-Plataforma (`package-product.ps1`)

O script de empacotamento compila e consolida todos os artefatos nativos em pacotes auto-contidos para **Windows (x64/ARM64)**, **Linux (x64/ARM64)** e **macOS (x64/ARM64)**.

### 5.1 Estrutura do Pacote de Distribuição v0.5.0

```text
remote-desktop-controller-windows-x64-0.5.0/
├── local-daemon.exe        # Runtime de execução, libSQL e scheduler
├── mcp-gateway.exe         # Adaptador MCP stdio -> IPC
├── device-agent.exe        # Agente cliente para pareamento remoto
├── remote-service.exe      # Roteador de serviços WebSocket/OAuth
├── rdc-monitor.exe         # Monitor visual da telemetria de named pipe
├── setup-windows.ps1       # Script de inicialização guiada
├── manifest.json           # Metadados e hashes SHA-256 de cada arquivo
├── INSTALL.md              # Instruções de instalação sem compilação
├── OWNER.md                # Manual operacional do proprietário
└── LICENSES/               # Licenças de dependências incorporadas
```

### 5.2 Desinstalador Protegido (*Scoped Uninstaller*)
O script de desinstalação (`tools/distribution/uninstall.ps1` e raiz `uninstall.ps1`) possui verificação rígida de escopo de diretório:
- Jamais executa exclusões recursivas se o caminho de instalação for uma pasta de sistema (`C:\`, `C:\Windows`, `C:\Program Files`, diretório do usuário ou raiz de workspace).
- Garante o encerramento gracioso exclusivo dos processos do produto antes da limpeza dos arquivos, evitando danos colaterais a arquivos vizinhos.
