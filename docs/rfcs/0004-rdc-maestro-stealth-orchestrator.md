# RFC-0004 — RDC Maestro: Orquestrador Multi-Provedor Desktop e Motor Furtivo de Execução

**Status:** Aprovado & Implementado  
**Versão Alvo:** v0.5.0  
**Data:** 2026-10-13  
**Autor:** Remote Desktop Controller Core Team  
**Repositório:** https://github.com/Remote-Desktop-Controller/remotecontroller  

---

## 1. Sumário Executivo

Modelos de linguagem avançados (ChatGPT, Claude, Gemini, DeepSeek, Qwen) possuem forças complementares em raciocínio, escrita de código, auditoria de segurança e planejamento arquitetural. No entanto, depender de um único provedor via API paga introduz custos contínuos elevados, limites rígidos de taxa (*rate limits*) e pontos únicos de falha.

O RFC-0004 introduz o **RDC Maestro**, um aplicativo desktop nativo desenvolvido em **Tauri v2 e WebView2**, capaz de:
1. Orquestrar sessões web autenticadas dos principais provedores de IA simultaneamente.
2. Formar planos consensuais através de um **Blackboard Feed** colaborativo.
3. Executar ações e automações de interface utilizando um **Motor Furtivo de Biomecânica Humana (Curvas de Bézier e Injeção Estocástica)** que evita bloqueios por sistemas anti-bot.
4. Despachar planos aprovados pelo operador humano diretamente para o `Local Daemon` via Named Pipe dedicado (`\\.\pipe\rdc-agent-maestro`).

---

## 2. Arquitetura do RDC Maestro Desktop

```mermaid
flowchart TD
    subgraph UI Desktop ["RDC Maestro (Tauri v2 / React 19 / Tailwind v4)"]
        Composer[Mission Composer]
        Blackboard[Blackboard Feed & Consenso]
        ApproveBtn[Botão de Aprovação do Operador]
        Composer --> Blackboard
        Blackboard --> ApproveBtn
    end

    subgraph Provedores Web ["Sessões WebView2 Isoladas (EBWebView)"]
        W1[ChatGPT Webview]
        W2[Claude Webview]
        W3[Gemini Webview]
        W4[DeepSeek Webview]
        W5[Qwen Webview]
    end

    subgraph Motor Furtivo ["Stealth CDP Engine"]
        Sentinel[Challenge Sentinel: Turnstile / Cloudflare / CAPTCHA]
        Bezier[Biomecânica Bézier: Trajetórias Cinéticas com Tremor]
        Stochastic[Injeção Estocástica: Cadência Gaussiana de Teclas]
    end

    subgraph Runtime Local ["Local Daemon & Execution Kernel"]
        Pipe["Named Pipe: \\\\.\\pipe\\rdc-agent-maestro"]
        Kernel[Kernel::dispatch]
        Storage[(libSQL Audit & Event Graph)]
    end

    UI Desktop --> Provedores Web
    Provedores Web --> Motor Furtivo
    ApproveBtn -->|Plano Aprovado| Pipe
    Pipe --> Kernel
    Kernel --> Storage
```

---

## 3. Blackboard Feed e Mecanismo de Consenso Multi-LLM

O Maestro não trata provedores de IA como agentes isolados, mas como membros de um conselho consultivo:
1. **Disparo de Missão:** A missão formulada pelo operador no `MissionComposer` é distribuída aos provedores habilitados.
2. **Debate e Refinamento Cruzado:** As saídas de cada provedor são injetadas no Blackboard compartilhado. Um modelo pode auditar a solução proposta por outro (por exemplo, DeepSeek propondo a lógica algorítmica e Claude revisando segurança e edge cases).
3. **Síntese de Plano de Execução:** É gerado um plano estruturado contendo passos discretos com ferramentas exatas (`filesystem.write`, `process.spawn`, etc.).
4. **Gate de Aprovação Humana (*Human-in-the-Loop*):** O plano sintetizado **só é executado após clique explícito do operador humano** em `ApprovePlanButton`. A automação cega é terminantemente vedada.

---

## 4. Motor Furtivo de Execução Biomecânica (`stealth`)

Sistemas web modernos empregam análise comportamental contínua para distinguir operadores humanos de robôs automatizados (movimentos retilíneos perfeitos do cursor do mouse, intervalos fixos de milissegundos entre teclas, etc.). O subsistema `stealth` implementa fidelidade fisiológica real:

### 4.1 Trajetórias Suaves em Curvas de Bézier Cúbicas
A movimentação do cursor entre a posição inicial $P_0$ e o elemento alvo $P_3$ obedece a curvas de Bézier com pontos de controle intermediários $P_1$ e $P_2$ gerados dinamicamente com base na distância euclidiana:
$$B(t) = (1-t)^3 P_0 + 3(1-t)^2 t P_1 + 3(1-t) t^2 P_2 + t^3 P_3, \quad t \in [0, 1]$$

- **Perfil de Velocidade:** Aplica aceleração e desaceleração cinemática (função sino), mimetizando a inércia do braço e mão humana.
- **Micro-tremores Articulares:** Introduz ruído de baixa amplitude ao longo do trajeto simulando o tremor natural neuromuscular.

### 4.2 Injeção Estocástica de Teclas e Cliques
- Intervalos de pressionamento de tecla seguem distribuição normal calibrada ($60\text{ms} \pm 25\text{ms}$ por caractere).
- Tempo de retenção do clique (*mouse down* até *mouse up*) varia estocasticamente entre $80\text{ms}$ e $140\text{ms}$.

### 4.3 Sentinela de Desafios (`ChallengeSentinel`)
O `ChallengeSentinel` inspeciona continuamente o DOM em busca de assinaturas conhecidas de proteção contra robôs:
- Cloudflare Turnstile (`cf-turnstile-wrapper`, `challenges.cloudflare.com`).
- Google reCAPTCHA e hCaptcha.
- Telas de bloqueio por taxa excedida (*Rate Limit*).

Quando um desafio é detectado, a injeção automatizada é pausada imediatamente e o operador humano é notificado na janela principal para interagir manualmente com a interface antes de retomar a missão.

---

## 5. Canal de Ordens Integrado (`apps/local-daemon/src/maestro.rs`)

O RDC Maestro comunica o plano de execução aprovado com o daemon de sistema através de um Named Pipe privado:
- **Endpoint:** `\\.\pipe\rdc-agent-maestro`
- **Isolamento de Segurança:** O canal adota as mesmas proteções DACL do IPC do daemon (acesso restrito exclusivamente ao usuário corrente e ao `NT AUTHORITY\SYSTEM`).
- **Encaminhamento Direto para o Kernel:** Cada etapa do plano é recebida como requisição autenticada e enviada para `Kernel::dispatch`. Desta forma, todas as políticas de capability, logs em libSQL, redatores de segurança e telemetria funcionam sem qualquer desvio ou bypass.
