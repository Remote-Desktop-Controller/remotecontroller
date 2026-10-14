// RDC Maestro - Armazenamento Durável de Estado e Histórico de Missões
// Persiste as últimas 50 missões e o cabeçalho compactado de contexto em `%USERPROFILE%\.remotecontroller\maestro-missions.json`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

pub const MAX_PERSISTED_MISSIONS: usize = 50;

/// Registro durável de uma missão executada ou planejada pelo Maestro.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedMission {
    pub mission_id: String,
    pub prompt: String,
    pub status: String,
    #[serde(default = "default_turn_count")]
    pub turns_count: usize,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub consensus: Option<String>,
    #[serde(default)]
    pub last_checkpoint_id: Option<String>,
    #[serde(default)]
    pub context_log: Option<serde_json::Value>,
    #[serde(default)]
    pub execution_ack: Option<serde_json::Value>,
}

fn default_turn_count() -> usize {
    1
}

/// Conteúdo completo do arquivo durável de estado.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct StateStoreData {
    #[serde(default)]
    pub compacted_header: Option<String>,
    #[serde(default)]
    pub missions: Vec<PersistedMission>,
}

/// Gerenciador de persistência durável em disco com cache em memória sincronizado por `RwLock`.
#[derive(Clone)]
pub struct StateStore {
    path: PathBuf,
    data: Arc<RwLock<StateStoreData>>,
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StateStore {
    /// Inicializa o repositório apontando para o caminho padrão (`%USERPROFILE%\.remotecontroller\maestro-missions.json`).
    pub fn new() -> Self {
        let path = default_store_path();
        Self::with_path(path)
    }

    /// Inicializa o repositório com um caminho arbitrário (útil para testes em diretórios temporários).
    pub fn with_path(path: PathBuf) -> Self {
        let initial_data = Self::load_from_disk(&path).unwrap_or_default();
        Self {
            path,
            data: Arc::new(RwLock::new(initial_data)),
        }
    }

    /// Retorna o caminho do arquivo em disco.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Obtém todo o histórico de missões ordenado da mais recente para a mais antiga.
    pub async fn get_history(&self) -> Vec<PersistedMission> {
        let lock = self.data.read().await;
        let mut list = lock.missions.clone();
        list.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        list
    }

    /// Obtém uma missão específica pelo identificador.
    pub async fn get_mission(&self, mission_id: &str) -> Option<PersistedMission> {
        let lock = self.data.read().await;
        lock.missions.iter().find(|m| m.mission_id == mission_id).cloned()
    }

    /// Obtém o cabeçalho compactado de sessões anteriores, se existir.
    pub async fn get_compacted_header(&self) -> Option<String> {
        let lock = self.data.read().await;
        lock.compacted_header.clone()
    }

    /// Atualiza ou limpa o cabeçalho compactado de contexto.
    pub async fn set_compacted_header(&self, header: Option<String>) -> Result<(), String> {
        let mut lock = self.data.write().await;
        lock.compacted_header = header;
        self.save_to_disk(&lock)
    }

    /// Registra uma nova missão ou atualiza uma já existente pelo `mission_id`.
    /// Mantém no máximo `MAX_PERSISTED_MISSIONS` (FIFO).
    pub async fn record_mission(&self, mission: PersistedMission) -> Result<(), String> {
        let mut lock = self.data.write().await;
        if let Some(pos) = lock.missions.iter().position(|m| m.mission_id == mission.mission_id) {
            lock.missions[pos] = mission;
        } else {
            lock.missions.push(mission);
        }

        // Ordena por started_at e mantém as 50 mais recentes
        lock.missions.sort_by_key(|m| m.started_at);
        if lock.missions.len() > MAX_PERSISTED_MISSIONS {
            let drain_count = lock.missions.len() - MAX_PERSISTED_MISSIONS;
            lock.missions.drain(0..drain_count);
        }

        self.save_to_disk(&lock)
    }

    /// Atualiza o status e recibo de execução de uma missão existente.
    pub async fn update_execution_result(
        &self,
        mission_id: &str,
        status: &str,
        execution_ack: Option<serde_json::Value>,
        checkpoint_id: Option<String>,
    ) -> Result<bool, String> {
        let mut lock = self.data.write().await;
        if let Some(m) = lock.missions.iter_mut().find(|m| m.mission_id == mission_id) {
            m.status = status.to_string();
            if let Some(ack) = execution_ack {
                m.execution_ack = Some(ack);
            }
            if let Some(cp) = checkpoint_id {
                m.last_checkpoint_id = Some(cp);
            }
            self.save_to_disk(&lock)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ----------------------------------------------------------------------------------
    // Operações em disco
    // ----------------------------------------------------------------------------------

    fn load_from_disk(path: &PathBuf) -> Option<StateStoreData> {
        if !path.exists() {
            return None;
        }
        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<StateStoreData>(&content) {
                Ok(data) => Some(data),
                Err(err) => {
                    warn!("Falha ao desserializar {}: {err}. Iniciando vazio.", path.display());
                    None
                }
            },
            Err(err) => {
                warn!("Falha ao ler {}: {err}. Iniciando vazio.", path.display());
                None
            }
        }
    }

    fn save_to_disk(&self, data: &StateStoreData) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("Falha ao criar diretório do store {}: {e}", parent.display())
                })?;
            }
        }

        let serialized = serde_json::to_string_pretty(data)
            .map_err(|e| format!("Falha ao serializar StateStore: {e}"))?;

        // Gravação segura via arquivo temporário para evitar corrupção em caso de desligamento abrupto
        let temp_path = self.path.with_extension("tmp");
        fs::write(&temp_path, serialized)
            .map_err(|e| format!("Falha ao escrever em {}: {e}", temp_path.display()))?;

        // No Windows, replace atômico exige remover ou sobrescrever
        if self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
        fs::rename(&temp_path, &self.path).map_err(|e| {
            format!(
                "Falha ao renomear {} para {}: {e}",
                temp_path.display(),
                self.path.display()
            )
        })?;

        info!("Estado do Maestro gravado com sucesso em {}", self.path.display());
        Ok(())
    }
}

/// Resolve o caminho padrão `%USERPROFILE%\.remotecontroller\maestro-missions.json`.
pub fn default_store_path() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join(".remotecontroller").join("maestro-missions.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn state_store_lifecycle_and_pruning() {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("maestro_test_{}", unique_id));
        let _ = std::fs::create_dir_all(&temp_dir);
        let store_path = temp_dir.join(".remotecontroller").join("maestro-missions.json");

        let store = StateStore::with_path(store_path.clone());

        // 1. Inicia vazio
        assert_eq!(store.get_history().await.len(), 0);
        assert_eq!(store.get_compacted_header().await, None);

        // 2. Define cabeçalho compactado
        store
            .set_compacted_header(Some("Resumo de sessões anteriores".to_string()))
            .await
            .unwrap();
        assert_eq!(
            store.get_compacted_header().await.as_deref(),
            Some("Resumo de sessões anteriores")
        );

        // 3. Grava 55 missões e verifica retenção de no máximo 50
        for i in 1..=55 {
            let mission = PersistedMission {
                mission_id: format!("m-{}", i),
                prompt: format!("Prompt {}", i),
                status: "completed".to_string(),
                turns_count: 1,
                started_at: 1000 + i as u64,
                finished_at: Some(1010 + i as u64),
                consensus: Some(format!("Consensus {}", i)),
                last_checkpoint_id: Some(format!("cp-{}", i)),
                context_log: None,
                execution_ack: None,
            };
            store.record_mission(mission).await.unwrap();
        }

        let history = store.get_history().await;
        assert_eq!(history.len(), MAX_PERSISTED_MISSIONS);
        // A mais recente é m-55
        assert_eq!(history[0].mission_id, "m-55");
        // A mais antiga sobrevivente é m-6 (1..=5 foram descartadas)
        assert_eq!(history.last().unwrap().mission_id, "m-6");

        // 4. Reidratação de nova instância a partir do mesmo caminho
        let rehydrated = StateStore::with_path(store_path);
        assert_eq!(rehydrated.get_history().await.len(), MAX_PERSISTED_MISSIONS);
        assert_eq!(
            rehydrated.get_compacted_header().await.as_deref(),
            Some("Resumo de sessões anteriores")
        );

        // 5. Atualização de status e checkpoint
        let updated = rehydrated
            .update_execution_result("m-55", "rolled_back", None, Some("cp-restored".into()))
            .await
            .unwrap();
        assert!(updated);
        let m55 = rehydrated.get_mission("m-55").await.unwrap();
        assert_eq!(m55.status, "rolled_back");
        assert_eq!(m55.last_checkpoint_id.as_deref(), Some("cp-restored"));
    }
}
