// RDC Maestro - Universal Webview Preload Script
// Injected into ChatGPT, Claude, and Gemini headless sessions.
//
// Fase 1: detecção de autenticação e injeção de prompt.
// Fase 3: captura do texto lido do último turno do agente e devolução ao motor
//         blackboard via o comando IPC `maestro_receive_reply`.

(function () {
  // 1. Executa exclusivamente na janela raiz (top-level). Jamais em iframes ou sub-frames.
  try {
    if (window !== window.top) {
      return;
    }
  } catch (_) {
    return;
  }

  const INIT_SYM = Symbol.for("rdc.maestro.init");
  if (window[INIT_SYM]) {
    return;
  }
  try {
    Object.defineProperty(window, INIT_SYM, {
      value: true,
      enumerable: false,
      configurable: false,
      writable: false
    });
  } catch (_) {}

  // Mascara a presença de window.chrome.webview contra inspeções superficiais de WAFs
  try {
    if (window.chrome && window.chrome.webview) {
      Object.defineProperty(window.chrome, "webview", {
        enumerable: false,
        configurable: true,
        get: () => undefined
      });
    }
  } catch (_) {}

  const hostname = window.location.hostname;
  let provider = "unknown";
  if (hostname.includes("chatgpt.com") || hostname.includes("openai.com")) {
    provider = "chatgpt";
  } else if (hostname.includes("claude.ai") || hostname.includes("anthropic.com")) {
    provider = "claude";
  } else if (
    hostname.includes("gemini.google.com") ||
    hostname.includes("aistudio.google.com") ||
    (hostname.includes("google.com") && window.location.pathname.startsWith("/app"))
  ) {
    provider = "gemini";
  } else if (hostname.includes("deepseek.com")) {
    provider = "deepseek";
  } else if (hostname.includes("qwen.ai") || hostname.includes("qwenlm.ai")) {
    provider = "qwen";
  }

  // Se não for um dos provedores suportados (ex: tela de OAuth do Google ou redirects externos),
  // encerra a execução do preload para não interferir no fluxo de autenticação nem poluir o console.
  if (provider === "unknown") {
    return;
  }

  // Ajustes do ciclo de captura de resposta (Fase 3).
  const CAPTURE = {
    pollMs: 900,
    settlePolls: 2,
    maxWaitMs: 110000,
    partialEveryMs: 4000,
    maxChars: 16000,
    // Idle debounce (ChatGPT): texto > idleMinChars inalterado por idleSettleMs => resposta concluída.
    idleMinChars: 20,
    idleSettleMs: 2000
  };

  /// Invoca um comando Tauri a partir de uma WebView remota sem expor a API globalmente.
  function invokeTauri(command, args) {
    const internals = window.__TAURI_INTERNALS__;
    if (internals && typeof internals.invoke === "function") {
      return internals.invoke(command, args);
    }
    const globalApi = window.__TAURI__;
    if (
      globalApi &&
      globalApi.core &&
      typeof globalApi.core.invoke === "function"
    ) {
      return globalApi.core.invoke(command, args);
    }
    return Promise.reject(
      new Error("IPC do Tauri indisponível nesta WebView (verifique a capability provider-bridge).")
    );
  }

  function textOf(element) {
    if (!element) {
      return "";
    }
    return (element.innerText || element.textContent || "").trim();
  }

  function cleanTurnText(element) {
    if (!element) {
      return "";
    }
    if (typeof element.cloneNode === "function") {
      try {
        const clone = element.cloneNode(true);
        const noise = clone.querySelectorAll
          ? clone.querySelectorAll(
              'button, nav, header, footer, .sr-only, [data-testid*="turn-action"], script, style, svg'
            )
          : [];
        Array.from(noise).forEach((node) => {
          if (typeof node.remove === "function") {
            node.remove();
          } else if (node.parentNode) {
            node.parentNode.removeChild(node);
          }
        });
        const cleaned = textOf(clone)
          .replace(/^(ChatGPT said:|O ChatGPT disse:)\s*/i, "")
          .trim();
        if (cleaned.length > 0) {
          return cleaned;
        }
      } catch (_) {
        // Fallback para leitura direta caso cloneNode falhe.
      }
    }
    return textOf(element)
      .replace(/^(ChatGPT said:|O ChatGPT disse:)\s*/i, "")
      .trim();
  }

  function compactText(value) {
    return String(value || "").replace(/\s+/g, "");
  }

  function hasInjectedText(element, expectedText) {
    const expected = compactText(expectedText);
    if (!expected) {
      return true;
    }
    const actual = compactText(textOf(element));
    return actual.length > 0 && actual.includes(expected);
  }

  function escapeHtml(raw) {
    return String(raw || "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function isElementActive(element) {
    if (!element || element.disabled || element.hidden) {
      return false;
    }
    if (
      typeof element.getAttribute === "function" &&
      (element.getAttribute("aria-hidden") === "true" ||
        element.getAttribute("aria-disabled") === "true")
    ) {
      return false;
    }

    let current = element;
    while (current && (current.nodeType === 1 || current === element)) {
      if (
        current.hidden ||
        (typeof current.getAttribute === "function" &&
          current.getAttribute("aria-hidden") === "true")
      ) {
        return false;
      }
      if (typeof window.getComputedStyle === "function") {
        try {
          const style = window.getComputedStyle(current);
          if (
            style &&
            (style.display === "none" ||
              style.visibility === "hidden" ||
              style.visibility === "collapse")
          ) {
            return false;
          }
        } catch (_) {
          // Ignora falhas de getComputedStyle em ambientes sintéticos.
        }
      }
      current = current.parentElement || null;
    }
    return true;
  }

  function placeCaretAtEnd(element) {
    if (!element || typeof window.getSelection !== "function" || typeof document.createRange !== "function") {
      return;
    }
    try {
      const selection = window.getSelection();
      if (!selection) {
        return;
      }
      const targetNode =
        element.lastElementChild ||
        (typeof element.querySelector === "function" ? element.querySelector("p") : null) ||
        element;
      const range = document.createRange();
      range.selectNodeContents(targetNode);
      if (typeof range.collapse === "function") {
        range.collapse(false);
      }
      selection.removeAllRanges();
      selection.addRange(range);
    } catch (_) {
      // Ignora falhas de Selection API em ambientes restritos.
    }
  }

  function dispatchEnterKey(target) {
    if (!target) {
      return;
    }
    if (typeof target.focus === "function") {
      try {
        target.focus();
      } catch (_) {}
    }
    const eventInit = {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
      bubbles: true,
      cancelable: true
    };
    target.dispatchEvent(new KeyboardEvent("keydown", eventInit));
    target.dispatchEvent(new KeyboardEvent("keypress", eventInit));
    target.dispatchEvent(new KeyboardEvent("keyup", eventInit));
  }

  function dispatchSimulatedInput(element, rawText, type) {
    try {
      if (typeof InputEvent === "function") {
        element.dispatchEvent(
          new InputEvent(type, {
            bubbles: true,
            cancelable: type === "beforeinput",
            inputType: "insertText",
            data: rawText
          })
        );
        return;
      }
    } catch (_) {
      // Fallback para Event simples caso InputEvent não seja suportado.
    }
    element.dispatchEvent(new Event(type, { bubbles: true, cancelable: type === "beforeinput" }));
  }

  /// Escreve texto em um editor (textarea/input ou contenteditable) disparando os eventos
  /// nativos que React/ProseMirror/Quill exigem para registrar a mudança.
  function typeIntoEditor(element, text) {
    const rawText = String(text ?? "");
    element.focus();
    try {
      element.dispatchEvent(new FocusEvent("focus", { bubbles: false }));
      element.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
    } catch (_) {
      element.dispatchEvent(new Event("focus"));
    }

    const isFormField =
      element.tagName === "TEXTAREA" || element.tagName === "INPUT";

    if (isFormField) {
      const prototype = Object.getPrototypeOf(element);
      const setter = prototype ? Object.getOwnPropertyDescriptor(prototype, "value") : null;
      if (setter && typeof setter.set === "function") {
        setter.set.call(element, rawText);
      } else {
        element.value = rawText;
      }
      dispatchSimulatedInput(element, rawText, "input");
      element.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    }

    // Contenteditable / ProseMirror: posiciona a seleção no parágrafo interno (ou no próprio
    // editor) com foco prévio e tenta document.execCommand('insertText', false, promptText).
    let insertedByExecCommand = false;
    let nativeInputDispatched = false;
    const origDispatch =
      typeof element.dispatchEvent === "function" ? element.dispatchEvent : null;
    if (origDispatch) {
      element.dispatchEvent = function (ev) {
        if (ev && ev.type === "input") {
          nativeInputDispatched = true;
        }
        return origDispatch.apply(this, arguments);
      };
    }

    try {
      const hasExistingContent = textOf(element).length > 0;
      const targetNode = hasExistingContent
        ? element
        : (typeof element.querySelector === "function" && element.querySelector("p")) || element;
      if (typeof window.getSelection === "function" && typeof document.createRange === "function") {
        const selection = window.getSelection();
        if (selection) {
          const range = document.createRange();
          range.selectNodeContents(targetNode);
          selection.removeAllRanges();
          selection.addRange(range);
        }
      }
      if (typeof document.execCommand === "function") {
        insertedByExecCommand = Boolean(document.execCommand("insertText", false, rawText));
      }
    } catch (err) {
      console.warn("[RDC-MAESTRO] execCommand insertText falhou, usando fallback:", err);
    } finally {
      if (origDispatch) {
        element.dispatchEvent = origDispatch;
      }
    }

    // No ProseMirror (Claude), o handler de `beforeinput` pode cancelar o comando fazendo
    // `execCommand` retornar `true` mesmo com o editor ainda vazio (especialmente em janela
    // oculta). Validamos o conteúdo real e aplicamos manipulação direta de innerHTML +
    // InputEvent simulado sempre que o texto não tiver sido efetivamente registrado.
    if (!insertedByExecCommand || !hasInjectedText(element, rawText)) {
      dispatchSimulatedInput(element, rawText, "beforeinput");
      if (!hasInjectedText(element, rawText)) {
        const paragraphsHtml = rawText
          .split("\n")
          .map((line) => `<p>${line ? escapeHtml(line) : "<br>"}</p>`)
          .join("");
        element.innerHTML = paragraphsHtml || `<p>${escapeHtml(rawText)}</p>`;
        placeCaretAtEnd(element);
      }
      dispatchSimulatedInput(element, rawText, "input");
      element.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    }

    if (!nativeInputDispatched) {
      dispatchSimulatedInput(element, rawText, "input");
    }
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }

  const CHATGPT_STOP_SELECTOR =
    'button[data-testid="stop-button"], button[aria-label*="Stop"], button[aria-label*="Parar"], button[aria-label*="Interromper"]';

  const CHATGPT_ACTION_SELECTOR = [
    'button[data-testid="copy-turn-action-button"]',
    'button[data-testid="good-response-turn-action-button"]',
    'button[data-testid="bad-response-turn-action-button"]',
    'button[data-testid*="turn-action-button"]',
    'button[aria-label*="Copy"]',
    'button[aria-label*="Copiar"]',
    'button[aria-label*="Good response"]',
    'button[aria-label*="Bad response"]',
    'button[aria-label*="Like"]',
    'button[aria-label*="Dislike"]',
    'button[aria-label*="Curtir"]',
    'button[aria-label*="Gostei"]',
    'button[aria-label*="Não gostei"]',
    'button[aria-label*="Boa resposta"]',
    'button[aria-label*="Regenerate"]',
    'button[aria-label*="Regenerar"]'
  ].join(", ");

  const MaestroBridge = {
    provider: provider,
    authState: "unknown",
    authNotified: false,
    replyCallback: null,
    lastExtractedResponse: "",
    lastPromptText: "",
    isStreaming: false,

    // Missão corrente carimbada pelo motor Rust antes da injeção do prompt.
    activeMission: null,

    capture: {
      mission: null,
      timer: null,
      lastText: "",
      stableCount: 0,
      startedAt: 0,
      finalSent: false,
      lastPartialAt: 0,
      lastTickAt: 0,
      sawStreaming: false,
      baselineTurnCount: 0,
      baselineText: "",
      idleText: "",
      idleSince: 0
    },

    detectAuth() {
      const href = window.location.href;

      if (this.provider === "chatgpt") {
        // Logged out markers
        if (href.includes("/auth/login") || href.includes("/login")) {
          this.authNotified = false;
          return { authenticated: false, detail: "Tela de login do ChatGPT detectada" };
        }
        // Logged in markers: modern ChatGPT prompt area or sidebar user profile
        const promptArea = document.querySelector('#prompt-textarea, div[contenteditable="true"], textarea, [data-placeholder]');
        const userMenu = document.querySelector("[data-testid='user-menu-button'], button[aria-label='User menu']");
        if (promptArea || userMenu) {
          if (!this.authNotified) {
            this.authNotified = true;
            try {
              invokeTauri('set_provider_connected', { provider: 'chatgpt' });
            } catch (err) {
              console.warn("[RDC-MAESTRO] Falha ao invocar set_provider_connected:", err);
            }
          }
          return { authenticated: true, detail: "Sessão ChatGPT autenticada (Prompt pronto)" };
        }
        return { authenticated: false, detail: "Aguardando autenticação na interface do ChatGPT" };
      }

      if (this.provider === "claude") {
        if (href.includes("/login") || href.includes("/auth")) {
          this.authNotified = false;
          return { authenticated: false, detail: "Tela de login do Claude detectada" };
        }
        const editor = document.querySelector('div[contenteditable="true"].ProseMirror, div[contenteditable="true"]');
        const accountButton = document.querySelector("[data-testid='user-menu'], button[aria-label*='Account']");
        if (editor || accountButton) {
          if (!this.authNotified) {
            this.authNotified = true;
            try {
              invokeTauri('set_provider_connected', { provider: 'claude' });
            } catch (err) {
              console.warn("[RDC-MAESTRO] Falha ao invocar set_provider_connected:", err);
            }
          }
          return { authenticated: true, detail: "Sessão Claude autenticada (Editor pronto)" };
        }
        return { authenticated: false, detail: "Aguardando autenticação na interface do Claude" };
      }

      if (this.provider === "gemini") {
        if (href.includes("accounts.google.com")) {
          this.authNotified = false;
          return { authenticated: false, detail: "Tela de login Google detectada" };
        }
        const geminiInput = document.querySelector(".ql-editor, rich-textarea, textarea[aria-label*='prompt']");
        const mainUi = document.querySelector("chat-app, bard-sidenav, .chat-container");
        if (geminiInput || mainUi) {
          if (!this.authNotified) {
            this.authNotified = true;
            try {
              invokeTauri('set_provider_connected', { provider: 'gemini' });
            } catch (err) {
              console.warn("[RDC-MAESTRO] Falha ao invocar set_provider_connected:", err);
            }
          }
          return { authenticated: true, detail: "Sessão Gemini autenticada (Chat pronto)" };
        }
        return { authenticated: false, detail: "Aguardando autenticação na conta Google/Gemini" };
      }

      if (this.provider === "deepseek") {
        if (href.includes("/login") || href.includes("/sign_in")) {
          this.authNotified = false;
          return { authenticated: false, detail: "Tela de login do DeepSeek detectada" };
        }
        const dsInput = document.querySelector('textarea#chat-input, textarea, div[contenteditable="true"], .ds-chat-input');
        const dsUser = document.querySelector('[data-testid="user-profile"], .ds-user-avatar, button[aria-label*="User"]');
        if (dsInput || dsUser) {
          if (!this.authNotified) {
            this.authNotified = true;
            try {
              invokeTauri('set_provider_connected', { provider: 'deepseek' });
            } catch (err) {
              console.warn("[RDC-MAESTRO] Falha ao invocar set_provider_connected:", err);
            }
          }
          return { authenticated: true, detail: "Sessão DeepSeek autenticada (Chat pronto)" };
        }
        return { authenticated: false, detail: "Aguardando autenticação na conta DeepSeek" };
      }

      if (this.provider === "qwen") {
        if (href.includes("/login") || href.includes("/auth")) {
          this.authNotified = false;
          return { authenticated: false, detail: "Tela de login do Qwen detectada" };
        }
        const qwenInput = document.querySelector('textarea, div[contenteditable="true"], .ant-input, [data-placeholder]');
        const qwenUser = document.querySelector('.user-avatar, .ant-avatar, [data-testid="user-avatar"]');
        if (qwenInput || qwenUser) {
          if (!this.authNotified) {
            this.authNotified = true;
            try {
              invokeTauri('set_provider_connected', { provider: 'qwen' });
            } catch (err) {
              console.warn("[RDC-MAESTRO] Falha ao invocar set_provider_connected:", err);
            }
          }
          return { authenticated: true, detail: "Sessão Qwen autenticada (Chat pronto)" };
        }
        return { authenticated: false, detail: "Aguardando autenticação na conta Qwen" };
      }

      return { authenticated: false, detail: "Provedor desconhecido" };
    },

    checkAuth() {
      const res = this.detectAuth();
      const previousState = this.authState;
      this.authState = res.authenticated ? "authenticated" : "authenticating";

      if (previousState !== this.authState && this.provider !== "unknown") {
        console.log(`[RDC-MAESTRO] Transição de Auth para ${this.provider}: ${this.authState}`);
      }

      return {
        provider: this.provider,
        authenticated: res.authenticated,
        state: this.authState,
        url: window.location.href,
        detail: res.detail
      };
    },

    /**
     * Localiza o editor de prompt do provedor corrente.
     */
    promptEditor() {
      if (this.provider === "chatgpt") {
        return (
          document.querySelector("#prompt-textarea") ||
          document.querySelector('div[contenteditable="true"].ProseMirror') ||
          document.querySelector('div[contenteditable="true"]') ||
          document.querySelector('textarea[data-id="root"], textarea, [data-placeholder]')
        );
      }
      if (this.provider === "claude") {
        const proseMirrorCandidates = Array.from(
          document.querySelectorAll('div[contenteditable="true"].ProseMirror')
        );
        const activeProseMirror = proseMirrorCandidates.find((el) => isElementActive(el));
        if (activeProseMirror) {
          return activeProseMirror;
        }
        const proseMirror =
          proseMirrorCandidates[0] ||
          document.querySelector('div[contenteditable="true"].ProseMirror');
        if (proseMirror) {
          return proseMirror;
        }
        const editableCandidates = Array.from(
          document.querySelectorAll('div[contenteditable="true"]')
        );
        return (
          editableCandidates.find((el) => isElementActive(el)) ||
          editableCandidates[0] ||
          document.querySelector('div[contenteditable="true"]')
        );
      }
      if (this.provider === "gemini") {
        return document.querySelector(".ql-editor, rich-textarea .ql-editor, textarea[aria-label*='prompt'], textarea");
      }
      if (this.provider === "deepseek") {
        return document.querySelector('textarea#chat-input, textarea, div[contenteditable="true"], .ds-chat-input');
      }
      if (this.provider === "qwen") {
        return document.querySelector('textarea, div[contenteditable="true"].ProseMirror, div[contenteditable="true"], .ant-input');
      }
      return null;
    },

    /**
     * Botão de envio do provedor corrente, quando identificável.
     */
    sendButton() {
      let selectorGroups = [];
      if (this.provider === "chatgpt") {
        selectorGroups = [
          'button[data-testid="send-button"]',
          'button[data-testid*="send"]',
          'button[aria-label="Send prompt"]',
          'button[aria-label*="Send"], button[aria-label*="Enviar"]'
        ];
      } else if (this.provider === "claude") {
        selectorGroups = [
          'button[aria-label="Send Message"], button[aria-label="Send message"], button[data-testid="send-button"]',
          'button[aria-label*="Send"], button[aria-label*="Enviar"]'
        ];
      } else if (this.provider === "gemini") {
        selectorGroups = [
          "button.send-button",
          'button[aria-label*="Send"], button[aria-label*="Enviar"]',
          '.send-button-container button',
          'button[data-test-id*="send"]',
          'button.send-button-mat'
        ];
      } else if (this.provider === "deepseek") {
        selectorGroups = [
          'div[role="button"][aria-disabled="false"]',
          '.ds-send-button',
          'button[aria-label*="Send"], button[aria-label*="Enviar"]',
          'button[type="submit"]'
        ];
      } else if (this.provider === "qwen") {
        selectorGroups = [
          'button.ant-btn-primary',
          'button[aria-label*="Send"], button[aria-label*="Enviar"]',
          'button[type="submit"]'
        ];
      }

      const candidates = [];
      for (const selector of selectorGroups) {
        try {
          const found = Array.from(document.querySelectorAll(selector));
          for (const btn of found) {
            if (!candidates.includes(btn)) {
              candidates.push(btn);
            }
          }
        } catch (_) {}
      }

      const active = candidates.find((btn) => isElementActive(btn));
      return active || candidates[0] || null;
    },

    sendPrompt(promptText, missionId) {
      if (missionId) {
        this.activeMission = missionId;
      }
      const normalizedPrompt = String(promptText ?? "");
      console.log(`[RDC-MAESTRO] Injetando prompt para ${this.provider}:`, normalizedPrompt.substring(0, 50));

      const editor = this.promptEditor();
      if (!editor) {
        throw new Error(`Editor de prompt do provedor ${this.provider} não encontrado.`);
      }

      const mission = missionId || this.activeMission || null;
      this.activeMission = mission;
      this.lastPromptText = normalizedPrompt.trim();
      this.startCapture(mission);

      typeIntoEditor(editor, normalizedPrompt);

      setTimeout(() => {
        const currentEditor = this.promptEditor() || editor;
        if (currentEditor && !hasInjectedText(currentEditor, normalizedPrompt)) {
          typeIntoEditor(currentEditor, normalizedPrompt);
        }

        const sendBtn = this.sendButton();
        if (sendBtn && isElementActive(sendBtn)) {
          sendBtn.click();
        } else {
          dispatchEnterKey(currentEditor);
        }

        // Caso o clique inicial ou o Enter não tenha esvaziado o editor (ex.: botão Send do
        // Claude/ChatGPT habilitou de forma assíncrona após re-injeção no ProseMirror),
        // realiza uma verificação adicional de submissão.
        setTimeout(() => {
          const postEditor = this.promptEditor() || currentEditor;
          if (postEditor && hasInjectedText(postEditor, normalizedPrompt) && !this.isGenerating()) {
            const retryBtn = this.sendButton();
            if (retryBtn && isElementActive(retryBtn)) {
              retryBtn.click();
            }
            if (hasInjectedText(postEditor, normalizedPrompt)) {
              dispatchEnterKey(postEditor);
            }
          }
        }, 120);
      }, 180);

      return true;
    },

    /**
     * Seletores (em ordem de preferência) do último turno do agente, por provedor.
     */
    replySelectors() {
      if (this.provider === "chatgpt") {
        return [
          'article[data-testid^="conversation-turn-"] [data-message-author-role="assistant"]',
          '[data-message-author-role="assistant"]',
          'article[data-testid^="conversation-turn-"] .markdown',
          'article[data-testid^="conversation-turn-"]',
          "div.agent-turn .markdown"
        ];
      }
      if (this.provider === "claude") {
        return [
          "div.font-claude-message",
          "div.font-claude-response",
          "[data-testid='assistant-message']",
          "div[data-is-streaming] .standard-markdown",
          "[data-message-author-role='assistant']"
        ];
      }
      if (this.provider === "gemini") {
        return [
          "model-response message-content",
          "model-response .markdown",
          "model-response .response-container-content",
          "model-response",
          "message-content",
          "[data-message-author-role='assistant']"
        ];
      }
      if (this.provider === "deepseek") {
        return [
          ".ds-markdown",
          ".ds-message-content",
          "[data-message-author-role='assistant']",
          ".markdown"
        ];
      }
      if (this.provider === "qwen") {
        return [
          "[data-message-author-role='assistant']",
          ".qwen-chat-message-assistant",
          ".chat-message-assistant",
          ".assistant-message",
          ".bot-message",
          ".qwen-chat-message:not(.qwen-chat-message-user)",
          ".message-content:not(.user-message-content)",
          ".markdown:not(.user-message .markdown)"
        ];
      }
      return ["[data-message-author-role='assistant']"];
    },

    /**
     * Identifica se um elemento `article[data-testid^="conversation-turn-"]` pertence ao usuário.
     */
    isChatGptUserTurn(article) {
      if (!article || typeof article.getAttribute !== "function") {
        return false;
      }
      const turnAttr = article.getAttribute("data-turn");
      const roleAttr = article.getAttribute("data-message-author-role");
      if (turnAttr === "user" || roleAttr === "user") {
        return true;
      }
      if (turnAttr === "assistant" || roleAttr === "assistant") {
        return false;
      }
      const hasAssistantRole =
        typeof article.querySelector === "function" &&
        Boolean(article.querySelector('[data-message-author-role="assistant"], .agent-turn'));
      if (hasAssistantRole) {
        return false;
      }
      const hasUserRole =
        typeof article.querySelector === "function" &&
        Boolean(article.querySelector('[data-message-author-role="user"], .user-turn'));
      if (hasUserRole) {
        return true;
      }
      if (typeof article.querySelector === "function") {
        const srOnly = article.querySelector(".sr-only, h5, h6");
        if (srOnly && /^(You said|Você disse|Dijiste):/i.test(textOf(srOnly))) {
          return true;
        }
      }
      return false;
    },

    /**
     * Retorna os elementos de turno do assistente no ChatGPT.
     */
    chatGptAssistantTurns() {
      const articles = Array.from(
        document.querySelectorAll('article[data-testid^="conversation-turn-"]')
      );
      if (articles.length > 0) {
        return articles.filter((article) => !this.isChatGptUserTurn(article));
      }

      return Array.from(document.querySelectorAll('[data-message-author-role="assistant"]'));
    },

    /**
     * Extrai o texto do último turno do ChatGPT a partir de `article[data-testid^="conversation-turn-"]`
     * ou `[data-message-author-role="assistant"]`.
     */
    extractChatGptResponse() {
      // 1. Procura diretamente elementos marcados explicitamente como resposta do assistente
      const directAssistantNodes = Array.from(
        document.querySelectorAll(
          '[data-message-author-role="assistant"], div.agent-turn, article[data-turn="assistant"]'
        )
      );
      for (let index = directAssistantNodes.length - 1; index >= 0; index -= 1) {
        const node = directAssistantNodes[index];
        const md = node.querySelector ? node.querySelector(".markdown") : null;
        const text = cleanTurnText(md || node);
        if (
          text.length > 0 &&
          (!this.lastPromptText || compactText(text) !== compactText(this.lastPromptText))
        ) {
          return text;
        }
      }

      // 2. Inspeciona artigos do ChatGPT ignorando turnos de usuário
      const articles = Array.from(
        document.querySelectorAll('article[data-testid^="conversation-turn-"], article')
      );
      for (let index = articles.length - 1; index >= 0; index -= 1) {
        const article = articles[index];
        if (this.isChatGptUserTurn(article)) {
          continue;
        }
        const md = article.querySelector ? article.querySelector(".markdown") : null;
        const text = cleanTurnText(md || article);
        if (
          text.length > 0 &&
          (!this.lastPromptText || compactText(text) !== compactText(this.lastPromptText))
        ) {
          return text;
        }
      }

      return "";
    },

    /**
     * Extrai o texto do último turno do agente no DOM.
     */
    extractResponse() {
      if (this.provider === "chatgpt") {
        return this.extractChatGptResponse();
      }

      for (const selector of this.replySelectors()) {
        let nodes;
        try {
          nodes = document.querySelectorAll(selector);
        } catch (err) {
          continue;
        }
        for (let index = nodes.length - 1; index >= 0; index -= 1) {
          const node = nodes[index];
          // Ignora se o elemento for pertencente a uma bolha de usuário
          if (
            node.closest &&
            node.closest(
              "user-query, .user-query, [data-message-author-role='user'], .qwen-chat-message-user, [data-turn='user']"
            )
          ) {
            continue;
          }
          const text = textOf(node);
          if (text.length > 0) {
            // Rejeita categoricamente se for o próprio texto do prompt do usuário
            if (this.lastPromptText && compactText(text) === compactText(this.lastPromptText)) {
              continue;
            }
            return text;
          }
        }
      }
      return "";
    },

    /**
     * Indica se o provedor ainda está gerando a resposta (streaming em andamento).
     */
    isGenerating() {
      const stopButtons = Array.from(document.querySelectorAll(CHATGPT_STOP_SELECTOR));
      if (stopButtons.some((btn) => isElementActive(btn))) {
        return true;
      }
      if (this.provider === "claude") {
        const streamingNodes = Array.from(
          document.querySelectorAll('[data-is-streaming="true"]')
        );
        if (streamingNodes.some((node) => isElementActive(node))) {
          return true;
        }
      }
      if (this.provider === "gemini") {
        const stopBtn = Array.from(
          document.querySelectorAll('button[aria-label*="Stop"], button[aria-label*="Parar"], .stop-button, mat-icon[data-mat-icon-name="stop"]')
        );
        if (stopBtn.some((btn) => isElementActive(btn))) {
          return true;
        }
      }
      if (this.provider === "deepseek") {
        const stopBtn = Array.from(
          document.querySelectorAll('.ds-stop-button, button[aria-label*="Stop"], button[aria-label*="Parar"]')
        );
        if (stopBtn.some((btn) => isElementActive(btn))) {
          return true;
        }
      }
      if (this.provider === "qwen") {
        const stopBtn = Array.from(
          document.querySelectorAll('button[aria-label*="Stop"], button[aria-label*="Parar"], .ant-btn-danger')
        );
        if (stopBtn.some((btn) => isElementActive(btn))) {
          return true;
        }
      }
      return false;
    },

    /**
     * Verifica se os botões de ação pós-resposta (copiar/curtir) já surgiram no último turno do ChatGPT.
     */
    hasActionButtons() {
      const filterValidAction = (buttons) =>
        buttons.some(
          (btn) =>
            isElementActive(btn) &&
            (typeof btn.closest !== "function" ||
              (!btn.closest("pre, code") &&
                !btn.closest('[data-turn="user"], [data-message-author-role="user"]')))
        );

      const articles = Array.from(
        document.querySelectorAll('article[data-testid^="conversation-turn-"]')
      );
      if (articles.length > 0) {
        let latestAssistantArticle = null;
        for (let index = articles.length - 1; index >= 0; index -= 1) {
          const article = articles[index];
          if (this.isChatGptUserTurn(article)) {
            break;
          }
          latestAssistantArticle = article;
          break;
        }
        if (!latestAssistantArticle || typeof latestAssistantArticle.querySelectorAll !== "function") {
          return false;
        }
        const turnButtons = Array.from(
          latestAssistantArticle.querySelectorAll(CHATGPT_ACTION_SELECTOR)
        );
        return filterValidAction(turnButtons);
      }

      const assistantTurns = Array.from(
        document.querySelectorAll('[data-message-author-role="assistant"]')
      );
      if (assistantTurns.length === 0) {
        return false;
      }

      const lastTurn = assistantTurns[assistantTurns.length - 1];
      if (typeof lastTurn.querySelectorAll === "function") {
        const turnButtons = Array.from(lastTurn.querySelectorAll(CHATGPT_ACTION_SELECTOR));
        if (filterValidAction(turnButtons)) {
          return true;
        }
      }

      const globalButtons = Array.from(document.querySelectorAll(CHATGPT_ACTION_SELECTOR));
      return filterValidAction(globalButtons);
    },

    /**
     * Inicia o ciclo de captura para a missão informada.
     */
    startCapture(missionId) {
      const capture = this.capture;
      this.stopCapture();

      capture.mission = missionId;
      capture.lastText = "";
      capture.stableCount = 0;
      capture.finalSent = false;
      capture.lastPartialAt = 0;
      capture.lastTickAt = 0;
      capture.idleText = "";
      capture.idleSince = 0;
      capture.sawStreaming = this.isGenerating();
      capture.baselineTurnCount =
        this.provider === "chatgpt" ? this.chatGptAssistantTurns().length : 0;
      capture.baselineText = this.extractResponse() || this.lastExtractedResponse || "";

      capture.startedAt = Date.now();
      capture.timer = setInterval(() => this.tickCapture(true), CAPTURE.pollMs);
      console.log(`[RDC-MAESTRO] Captura de resposta iniciada (missão: ${missionId || "sem-carimbo"}).`);
    },

    stopCapture() {
      const capture = this.capture;
      if (capture.timer) {
        clearInterval(capture.timer);
        capture.timer = null;
      }
    },

    /**
     * Verifica a estabilidade do texto extraído e decide quando consolidar a resposta.
     */
    tickCapture(fromInterval) {
      const capture = this.capture;
      if (capture.finalSent) {
        return;
      }

      const now = Date.now();
      const text = this.extractResponse();
      const streaming = this.isGenerating();
      this.isStreaming = streaming;
      if (streaming) {
        capture.sawStreaming = true;
      }

      const elapsed = capture.startedAt ? now - capture.startedAt : 0;
      const currentTurnCount =
        this.provider === "chatgpt" ? this.chatGptAssistantTurns().length : 0;
      const isStalePreviousTurn =
        Boolean(capture.baselineText) &&
        text === capture.baselineText &&
        currentTurnCount <= capture.baselineTurnCount;

      if (!text || isStalePreviousTurn) {
        capture.stableCount = 0;
        capture.idleText = "";
        capture.idleSince = 0;
        return;
      }

      const timedOut = elapsed >= CAPTURE.maxWaitMs;

      // Idle debounce: mede (por relógio, não por contagem de ticks) há quanto tempo o texto
      // do último turno permanece idêntico. Funciona mesmo que o stop-button fique "preso" no
      // DOM ou que os botões de ação nunca apareçam, e é imune à cadência do MutationObserver.
      if (text !== capture.idleText) {
        capture.idleText = text;
        capture.idleSince = now;
      }
      const idleSettled =
        text.length > CAPTURE.idleMinChars &&
        now - capture.idleSince >= CAPTURE.idleSettleMs;

      if (streaming && !timedOut && !idleSettled) {
        capture.lastText = text;
        capture.stableCount = 0;
        capture.lastTickAt = now;
        if (now - capture.lastPartialAt >= CAPTURE.partialEveryMs) {
          capture.lastPartialAt = now;
          this.submitCapture(text, true);
        }
        return;
      }

      const canAdvanceStability =
        Boolean(fromInterval) ||
        capture.lastTickAt === 0 ||
        now - capture.lastTickAt >= Math.floor(CAPTURE.pollMs / 2);

      if (text === capture.lastText) {
        if (canAdvanceStability) {
          capture.stableCount += 1;
          capture.lastTickAt = now;
        }
      } else {
        capture.lastText = text;
        capture.stableCount = 1;
        capture.finalSent = false;
        capture.lastTickAt = now;
      }

      // Gatilho imediato: botões de ação (copiar/curtir) surgiram e o stop-button sumiu.
      const chatGptTurnReady =
        this.provider === "chatgpt" &&
        !streaming &&
        this.hasActionButtons();

      if (
        !capture.finalSent &&
        (chatGptTurnReady ||
          idleSettled ||
          (!streaming && capture.stableCount >= CAPTURE.settlePolls) ||
          timedOut)
      ) {
        const textoFinal = this.extractResponse() || text;
        capture.finalSent = true;
        this.lastExtractedResponse = textoFinal;
        this.submitCapture(textoFinal, false);
        this.stopCapture();
      }
    },

    /**
     * Medição de telemetria do DOM e quantidade de turnos da conversa.
     */
    getDomMetrics() {
      const bodyText = (document.body && (document.body.innerText || document.body.textContent)) || "";
      const domChars = bodyText.length;
      let turnCount = 0;
      try {
        if (this.provider === "chatgpt") {
          turnCount = document.querySelectorAll('[data-message-author-role], article').length;
        } else if (this.provider === "claude") {
          turnCount = document.querySelectorAll('[data-test-render-count], .font-claude-message').length;
        } else if (this.provider === "gemini") {
          turnCount = document.querySelectorAll('message-content, .model-response-text').length;
        } else if (this.provider === "deepseek") {
          turnCount = document.querySelectorAll('.ds-message, [data-role]').length;
        } else if (this.provider === "qwen") {
          turnCount = document.querySelectorAll('.message-item, .chat-message').length;
        }
        if (turnCount === 0) {
          turnCount = document.querySelectorAll('article, [data-message-author-role]').length || 1;
        }
      } catch (_) {
        turnCount = 1;
      }
      return { domChars, turnCount };
    },

    /**
     * Inicia uma nova conversa limpa (Compact & Reset) clicando no botão da UI do SPA
     * ou navegando para a URL raiz como fallback.
     */
    startFreshChat() {
      console.log(`[RDC-MAESTRO] Reiniciando sessão e criando novo chat limpo para ${this.provider}...`);
      this.stopCapture();
      this.activeMission = null;

      let clicked = false;
      let selectors = [];
      if (this.provider === "chatgpt") {
        selectors = [
          'a[href="/"]',
          'button[aria-label="New chat"]',
          'button[aria-label="Novo chat"]',
          '[data-testid="create-new-chat-button"]'
        ];
      } else if (this.provider === "claude") {
        selectors = [
          'a[href="/new"]',
          'button[aria-label="Start new chat"]',
          '[data-testid="new-chat-button"]',
          'a[href^="/chat/"]'
        ];
      } else if (this.provider === "gemini") {
        selectors = [
          'button[aria-label*="New chat"]',
          'button[aria-label*="Nova conversa"]',
          'a[href="/app"]'
        ];
      } else if (this.provider === "deepseek") {
        selectors = [
          'a[href="/"]',
          'button[aria-label*="New Chat"]',
          'button[aria-label*="Novo Chat"]',
          '.ds-new-chat'
        ];
      } else if (this.provider === "qwen") {
        selectors = [
          'a[href="/"]',
          'button[aria-label*="New chat"]',
          'button[aria-label*="Novo chat"]'
        ];
      }

      for (const sel of selectors) {
        try {
          const btn = document.querySelector(sel);
          if (btn && typeof btn.click === "function") {
            btn.click();
            clicked = true;
            console.log(`[RDC-MAESTRO] Botão de novo chat acionado via seletor: ${sel}`);
            break;
          }
        } catch (_) {}
      }

      if (!clicked) {
        console.log(`[RDC-MAESTRO] Fallback de navegação SPA para novo chat em ${this.provider}.`);
        const origin = window.location.origin;
        if (this.provider === "gemini") {
          window.location.href = `${origin}/app`;
        } else if (this.provider === "claude") {
          window.location.href = `${origin}/new`;
        } else {
          window.location.href = `${origin}/`;
        }
      }
    },

    /**
     * Devolve o texto lido ao motor blackboard do Maestro (comando IPC).
     */
    submitCapture(text, partial) {
      const textoFinal = String(text || "").slice(0, CAPTURE.maxChars);
      if (!textoFinal.trim()) {
        return;
      }

      // Telemetria de DOM bloat (> 100KB)
      const metrics = this.getDomMetrics();
      if (metrics.domChars > 100000) {
        console.warn(
          `[RDC-MAESTRO] ALERTA: DOM Bloat excessivo detectado em ${this.provider} (${metrics.domChars} caracteres, ${metrics.turnCount} turnos). Recomendado acionar 'Compact & Novo Chat'.`
        );
      }

      const missionId = this.capture.mission || this.activeMission || null;

      invokeTauri("maestro_receive_reply", {
        provider: this.provider,
        text: textoFinal,
        mission: missionId,
        partial: Boolean(partial)
      })
        .then(() => {
          if (!partial) {
            console.log(`[RDC-MAESTRO] Resposta de ${this.provider} entregue ao motor blackboard.`);
            if (typeof this.replyCallback === "function") {
              this.replyCallback({ provider: this.provider, text: textoFinal, mission: missionId });
            }
          }
        })
        .catch((err) => {
          console.warn("[RDC-MAESTRO] Falha ao entregar resposta ao motor:", err);
        });
    },

    onReply(callback) {
      this.replyCallback = callback;
    }
  };

  // Receptor seguro de comandos via HTML5 Web Messaging sem poluir o escopo global `window`
  window.addEventListener("message", (event) => {
    if (!event || !event.data || event.data.source !== "rdc-maestro") {
      return;
    }
    const data = event.data;
    if (data.action === "send_prompt") {
      try {
        MaestroBridge.sendPrompt(data.prompt, data.mission_id);
      } catch (err) {
        console.warn("[RDC-MAESTRO] Falha ao injetar prompt:", err);
      }
    } else if (data.action === "check_auth") {
      MaestroBridge.checkAuth();
    } else if (data.action === "start_fresh_chat" || data.action === "reset_chat") {
      try {
        MaestroBridge.startFreshChat();
      } catch (err) {
        console.warn("[RDC-MAESTRO] Falha ao iniciar novo chat:", err);
      }
    }
  });

  // Observador Contínuo de DOM para detectar conclusão de login e término de streaming
  const observer = new MutationObserver(() => {
    MaestroBridge.checkAuth();
    if (MaestroBridge.capture.timer && !MaestroBridge.capture.finalSent) {
      if (MaestroBridge.isGenerating()) {
        MaestroBridge.capture.sawStreaming = true;
      } else if (
        (MaestroBridge.provider === "chatgpt" && MaestroBridge.hasActionButtons()) ||
        MaestroBridge.capture.sawStreaming
      ) {
        MaestroBridge.tickCapture(false);
      }
    }
  });

  const observerConfig = {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true
  };

  const observeTarget = document.documentElement || document.body;
  if (observeTarget) {
    observer.observe(observeTarget, observerConfig);
  } else {
    document.addEventListener("DOMContentLoaded", () => {
      const readyTarget = document.documentElement || document.body;
      if (readyTarget) {
        observer.observe(readyTarget, observerConfig);
      }
      MaestroBridge.checkAuth();
    });
  }

  // Exposição de API interna do Maestro para controle via WebView eval
  try {
    Object.defineProperty(window, "__MAESTRO_BRIDGE__", {
      value: MaestroBridge,
      enumerable: false,
      configurable: true,
      writable: true
    });
  } catch (_) {
    window.__MAESTRO_BRIDGE__ = MaestroBridge;
  }

  // Verificação inicial
  MaestroBridge.checkAuth();
})();
