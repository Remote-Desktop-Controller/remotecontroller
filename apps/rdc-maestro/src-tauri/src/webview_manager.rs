// RDC Maestro - Gerenciador de WebViews Headless e Sessões de IA
// Responsável por instanciar e manter o ciclo de vida das sessões do ChatGPT, Claude, Gemini, DeepSeek e Qwen.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::RwLock;
use tracing::{info, warn};

pub const PRELOAD_SCRIPT: &str = include_str!("preload.js");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderState {
    Disconnected,
    Authenticating,
    Connected,
}

impl std::fmt::Display for ProviderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderState::Disconnected => write!(f, "disconnected"),
            ProviderState::Authenticating => write!(f, "authenticating"),
            ProviderState::Connected => write!(f, "connected"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub url: String,
    pub state: ProviderState,
    pub visible: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub label: &'static str,
    pub title: &'static str,
}

pub const PROVIDERS: &[ProviderConfig] = &[
    ProviderConfig {
        id: "chatgpt",
        name: "ChatGPT",
        url: "https://chatgpt.com",
        label: "provider-chatgpt",
        title: "Sessão ChatGPT - RDC Maestro",
    },
    ProviderConfig {
        id: "claude",
        name: "Claude",
        url: "https://claude.ai",
        label: "provider-claude",
        title: "Sessão Claude - RDC Maestro",
    },
    ProviderConfig {
        id: "gemini",
        name: "Gemini",
        url: "https://gemini.google.com",
        label: "provider-gemini",
        title: "Sessão Gemini - RDC Maestro",
    },
    ProviderConfig {
        id: "deepseek",
        name: "DeepSeek",
        url: "https://chat.deepseek.com",
        label: "provider-deepseek",
        title: "Sessão DeepSeek - RDC Maestro",
    },
    ProviderConfig {
        id: "qwen",
        name: "Qwen",
        url: "https://chat.qwen.ai",
        label: "provider-qwen",
        title: "Sessão Qwen - RDC Maestro",
    },
];

/// Nomes dos provedores unidos por vírgula e pelo conector final (ex.: "A, B ou C").
/// Evita textos de UI/erro com a lista de provedores fixa no código.
pub fn provider_names_joined(connector: &str) -> String {
    let names: Vec<&str> = PROVIDERS.iter().map(|p| p.name).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!("{} {} {}", rest.join(", "), connector, last)
        }
        Some((last, _)) => (*last).to_string(),
        None => String::new(),
    }
}

pub struct WebviewManager {
    statuses: Arc<RwLock<HashMap<String, ProviderStatus>>>,
}

impl Default for WebviewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WebviewManager {
    pub fn new() -> Self {
        let mut map = HashMap::new();
        for p in PROVIDERS {
            map.insert(
                p.id.to_string(),
                ProviderStatus {
                    id: p.id.to_string(),
                    name: p.name.to_string(),
                    url: p.url.to_string(),
                    state: ProviderState::Disconnected,
                    visible: false,
                    detail: "Sessão não iniciada".to_string(),
                },
            );
        }

        Self {
            statuses: Arc::new(RwLock::new(map)),
        }
    }

    pub fn find_config(provider_id: &str) -> Option<&'static ProviderConfig> {
        PROVIDERS.iter().find(|p| p.id == provider_id)
    }

    /// Obtém uma janela existente ou cria uma nova com isolamento e script de preload injetado
    pub fn get_or_create_window(
        &self,
        app: &AppHandle,
        config: &'static ProviderConfig,
        initially_visible: bool,
    ) -> Result<WebviewWindow, String> {
        Self::ensure_window(app, config, initially_visible)
    }

    /// Variante associativa (sem `&self`) da obtenção/criação da janela.
    ///
    /// Usada pelo motor de orquestração (Fase 3), que precisa garantir a existência das
    /// WebViews ocultas antes de injetar o prompt de debate sem depender do estado da UI.
    pub fn ensure_window(
        app: &AppHandle,
        config: &'static ProviderConfig,
        initially_visible: bool,
    ) -> Result<WebviewWindow, String> {
        if let Some(window) = app.get_webview_window(config.label) {
            return Ok(window);
        }

        info!(
            "Instanciando WebviewWindow para [{}] ({}) em modo background",
            config.name, config.url
        );

        let parsed_url: url::Url = config
            .url
            .parse()
            .map_err(|e| format!("URL inválida para {}: {}", config.id, e))?;

        let profile_dir = app
            .path()
            .app_local_data_dir()
            .map_err(|e| format!("Falha ao resolver diretório local de dados: {}", e))?
            .join("EBWebView");

        if let Err(e) = std::fs::create_dir_all(&profile_dir) {
            warn!(
                "Não foi possível assegurar diretório de perfil persistente {:?}: {}",
                profile_dir, e
            );
        }

        let window = WebviewWindowBuilder::new(app, config.label, WebviewUrl::External(parsed_url))
            .title(config.title)
            .inner_size(1080.0, 780.0)
            .visible(initially_visible)
            .data_directory(profile_dir)
            .general_autofill_enabled(true)
            .on_new_window(|url, _features| {
                info!("Popup/Nova janela autorizada para OAuth/SSO: {}", url);
                tauri::webview::NewWindowResponse::Allow
            })
            .initialization_script(PRELOAD_SCRIPT)
            .build()
            .map_err(|e| format!("Falha ao construir WebviewWindow para {}: {}", config.id, e))?;

        Ok(window)
    }

    /// Abre a janela para login e inicia verificação contínua da autenticação
    pub async fn open_login(
        &self,
        app: &AppHandle,
        provider_id: &str,
    ) -> Result<ProviderStatus, String> {
        let config = Self::find_config(provider_id)
            .ok_or_else(|| format!("Provedor não reconhecido: {}", provider_id))?;

        let window = self.get_or_create_window(app, config, true)?;
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();

        let updated_status = {
            let mut lock = self.statuses.write().await;
            let status = lock.get_mut(provider_id).ok_or("Status não encontrado")?;
            status.state = ProviderState::Authenticating;
            status.visible = true;
            status.detail = "Janela aberta para autenticação do usuário".to_string();
            status.clone()
        };

        let _ = app.emit("provider-status-changed", &updated_status);

        // Dispara task de monitoramento da sessão
        let app_clone = app.clone();
        let statuses_clone = Arc::clone(&self.statuses);
        let pid = provider_id.to_string();
        let label = config.label.to_string();
        let name = config.name.to_string();

        tokio::spawn(async move {
            Self::monitor_session_auth(app_clone, statuses_clone, pid, label, name).await;
        });

        Ok(updated_status)
    }

    /// Oculta a janela mantendo a sessão e o DOM ativos em background
    pub async fn hide_window(
        &self,
        app: &AppHandle,
        provider_id: &str,
    ) -> Result<ProviderStatus, String> {
        let config = Self::find_config(provider_id)
            .ok_or_else(|| format!("Provedor não reconhecido: {}", provider_id))?;

        if let Some(window) = app.get_webview_window(config.label) {
            let _ = window.hide();
        }

        let updated_status = {
            let mut lock = self.statuses.write().await;
            let status = lock.get_mut(provider_id).ok_or("Status não encontrado")?;
            status.visible = false;
            status.clone()
        };

        let _ = app.emit("provider-status-changed", &updated_status);
        info!("Janela de [{}] ocultada com sucesso.", config.name);
        Ok(updated_status)
    }

    /// Exibe a janela para depuração ou intervenção manual
    pub async fn show_window(
        &self,
        app: &AppHandle,
        provider_id: &str,
    ) -> Result<ProviderStatus, String> {
        let config = Self::find_config(provider_id)
            .ok_or_else(|| format!("Provedor não reconhecido: {}", provider_id))?;

        let window = self.get_or_create_window(app, config, true)?;
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();

        let updated_status = {
            let mut lock = self.statuses.write().await;
            let status = lock.get_mut(provider_id).ok_or("Status não encontrado")?;
            status.visible = true;
            status.clone()
        };

        let _ = app.emit("provider-status-changed", &updated_status);
        Ok(updated_status)
    }

    /// Marca o provedor como conectado imediatamente, oculta a janela e emite atualização de estado
    pub async fn set_connected(
        &self,
        app: &AppHandle,
        provider_id: &str,
    ) -> Result<ProviderStatus, String> {
        let config = Self::find_config(provider_id)
            .ok_or_else(|| format!("Provedor não reconhecido: {}", provider_id))?;

        if let Some(window) = app.get_webview_window(config.label) {
            let _ = window.hide();
        }

        let updated_status = {
            let mut lock = self.statuses.write().await;
            let status = lock.get_mut(provider_id).ok_or("Status não encontrado")?;
            status.state = ProviderState::Connected;
            status.visible = false;
            status.detail = "Sessão autenticada. Operando em background.".to_string();
            status.clone()
        };

        let _ = app.emit("provider-status-changed", &updated_status);
        info!(
            "Provedor [{}] confirmado como conectado e ocultado em background.",
            config.name
        );
        Ok(updated_status)
    }

    /// Retorna o status de todos os provedores
    pub async fn get_all_statuses(&self) -> Vec<ProviderStatus> {
        let lock = self.statuses.read().await;
        let mut list: Vec<ProviderStatus> = lock.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Loop assíncrono que sonda a WebView via eval() até detectar que a sessão está autenticada
    async fn monitor_session_auth(
        app: AppHandle,
        statuses: Arc<RwLock<HashMap<String, ProviderStatus>>>,
        provider_id: String,
        label: String,
        name: String,
    ) {
        info!("Iniciando monitoramento de autenticação para [{}]", name);

        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(1500)).await;

            {
                let lock = statuses.read().await;
                #[allow(clippy::collapsible_if)]
                if let Some(status) = lock.get(&provider_id) {
                    if status.state == ProviderState::Connected {
                        info!(
                            "Sessão [{}] já confirmada como conectada. Encerrando monitoramento.",
                            name
                        );
                        break;
                    }
                }
            }

            let maybe_window = app.get_webview_window(&label);
            let Some(window) = maybe_window else {
                warn!("Janela {} não encontrada durante monitoramento.", label);
                break;
            };

            // Solicita verificação de autenticação ao script de preload via Web Messaging
            let eval_result = window.eval(
                r#"window.postMessage({ source: "rdc-maestro", action: "check_auth" }, "*");"#,
            );
            if let Err(e) = eval_result {
                warn!("Erro ao avaliar eval em {}: {}", label, e);
                continue;
            }
        }

        info!(
            "Finalizado ciclo de monitoramento de autenticação para [{}]",
            name
        );
    }
}
