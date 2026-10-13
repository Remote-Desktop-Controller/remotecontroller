//! Credentials vault protected by Windows DPAPI (`CryptProtectData`/`CryptUnprotectData`).
//!
//! Stores sensitive user tokens (OpenAI, Anthropic, Bearer tokens, custom credentials)
//! encrypted at rest under `~/.remotecontroller/.vault/secrets.bin`.
//! Ensures plaintext secrets never touch disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("DPAPI encryption error: {0}")]
    Encryption(String),
    #[error("DPAPI decryption error: {0}")]
    Decryption(String),
    #[error("Secret key '{0}' not found in vault")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, VaultError>;

/// Low-level DPAPI encryption for arbitrary byte slices.
#[cfg(windows)]
pub fn dpapi_encrypt(data: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
    };

    if data.is_empty() {
        return Ok(Vec::new());
    }

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };

    let mut entropy_blob = CRYPT_INTEGER_BLOB::default();
    let p_entropy = if let Some(ent) = entropy {
        entropy_blob.cbData = ent.len() as u32;
        entropy_blob.pbData = ent.as_ptr() as *mut u8;
        Some(&entropy_blob as *const CRYPT_INTEGER_BLOB)
    } else {
        None
    };

    let mut out_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &in_blob,
            windows::core::PCWSTR::null(),
            p_entropy,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
        .map_err(|e| VaultError::Encryption(format!("CryptProtectData failed: {e}")))?;

        if out_blob.pbData.is_null() {
            return Err(VaultError::Encryption(
                "CryptProtectData returned null pointer".into(),
            ));
        }

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let ciphertext = slice.to_vec();
        let _ = LocalFree(Some(HLOCAL(out_blob.pbData as *mut _)));
        Ok(ciphertext)
    }
}

/// Low-level DPAPI decryption for arbitrary ciphertext slices.
#[cfg(windows)]
pub fn dpapi_decrypt(cipher: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };

    if cipher.is_empty() {
        return Ok(Vec::new());
    }

    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: cipher.len() as u32,
        pbData: cipher.as_ptr() as *mut u8,
    };

    let mut entropy_blob = CRYPT_INTEGER_BLOB::default();
    let p_entropy = if let Some(ent) = entropy {
        entropy_blob.cbData = ent.len() as u32;
        entropy_blob.pbData = ent.as_ptr() as *mut u8;
        Some(&entropy_blob as *const CRYPT_INTEGER_BLOB)
    } else {
        None
    };

    let mut out_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &in_blob,
            None,
            p_entropy,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
        .map_err(|e| VaultError::Decryption(format!("CryptUnprotectData failed: {e}")))?;

        if out_blob.pbData.is_null() {
            return Err(VaultError::Decryption(
                "CryptUnprotectData returned null pointer".into(),
            ));
        }

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let plaintext = slice.to_vec();
        let _ = LocalFree(Some(HLOCAL(out_blob.pbData as *mut _)));
        Ok(plaintext)
    }
}

/// Non-Windows fallback (for cross-platform compatibility and tests).
#[cfg(not(windows))]
pub fn dpapi_encrypt(data: &[u8], _entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    // Obfuscate with fixed byte transformation for test environments
    Ok(data.iter().map(|b| b ^ 0xA5).collect())
}

/// Non-Windows fallback decryption.
#[cfg(not(windows))]
pub fn dpapi_decrypt(cipher: &[u8], _entropy: Option<&[u8]>) -> Result<Vec<u8>> {
    Ok(cipher.iter().map(|b| b ^ 0xA5).collect())
}

/// A DPAPI-encrypted credential vault backed by a persistent file.
#[derive(Debug, Clone)]
pub struct Vault {
    path: PathBuf,
    secrets: HashMap<String, String>,
}

impl Vault {
    /// Opens or creates a credential vault at the given path.
    ///
    /// If the file exists, it is read and decrypted. If not, an empty vault is initialized.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut secrets = HashMap::new();

        if path.exists() {
            let encrypted = std::fs::read(&path)?;
            if !encrypted.is_empty() {
                let decrypted = dpapi_decrypt(&encrypted, None)?;
                secrets = serde_json::from_slice(&decrypted)?;
            }
        }

        Ok(Self { path, secrets })
    }

    /// Retrieves a secret by its identifier.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.secrets.get(key).map(|s| s.as_str())
    }

    /// Stores or updates a secret key-value pair and persists the encrypted vault to disk.
    pub fn set(&mut self, key: impl Into<String>, secret: impl Into<String>) -> Result<()> {
        self.secrets.insert(key.into(), secret.into());
        self.save()
    }

    /// Removes a secret from the vault and persists changes to disk.
    pub fn remove(&mut self, key: &str) -> Result<bool> {
        let removed = self.secrets.remove(key).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Returns a sorted list of all key names stored in the vault.
    pub fn list_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.secrets.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Persists the vault to disk, encrypting all entries with DPAPI via an atomic write.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let serialized = serde_json::to_vec(&self.secrets)?;
        let encrypted = dpapi_encrypt(&serialized, None)?;

        // Atomic replacement using tempfile
        let temp_dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(temp_dir)?;
        use std::io::Write;
        temp.write_all(&encrypted)?;
        temp.flush()?;
        temp.persist(&self.path).map_err(|e| e.error)?;

        Ok(())
    }

    /// Returns the file path of this vault.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_dpapi_raw_roundtrip() {
        let secret = b"sk-ant-api03-abcdef1234567890_topsecret_token";
        let encrypted = dpapi_encrypt(secret, None).unwrap();
        assert_ne!(encrypted, secret, "Ciphertext must not match plaintext");

        let decrypted = dpapi_decrypt(&encrypted, None).unwrap();
        assert_eq!(
            decrypted, secret,
            "Decrypted text must match original plaintext"
        );
    }

    #[test]
    fn test_dpapi_with_entropy() {
        let secret = b"sk-proj-super-secret-openai-key-value";
        let entropy = b"unique-session-salt-8899";

        let encrypted = dpapi_encrypt(secret, Some(entropy)).unwrap();
        let decrypted = dpapi_decrypt(&encrypted, Some(entropy)).unwrap();
        assert_eq!(decrypted, secret);

        // Attempting to decrypt with wrong entropy must fail
        let wrong_entropy = b"wrong-salt";
        let result = dpapi_decrypt(&encrypted, Some(wrong_entropy));
        assert!(result.is_err(), "Decryption with wrong entropy must fail");
    }

    #[test]
    fn test_dpapi_empty_data() {
        let empty = b"";
        let encrypted = dpapi_encrypt(empty, None).unwrap();
        assert!(encrypted.is_empty());
        let decrypted = dpapi_decrypt(&encrypted, None).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_vault_lifecycle() {
        let tmp = tempdir().unwrap();
        let vault_path = tmp.path().join(".vault").join("secrets.bin");

        // 1. Create and populate vault
        {
            let mut vault = Vault::open(&vault_path).unwrap();
            vault.set("openai_api_key", "sk-proj-1234567890").unwrap();
            vault.set("anthropic_key", "sk-ant-9876543210").unwrap();
            assert_eq!(vault.get("openai_api_key"), Some("sk-proj-1234567890"));
            assert_eq!(vault.list_keys(), vec!["anthropic_key", "openai_api_key"]);
        }

        // Verify file is encrypted on disk (does not contain plaintext)
        let raw_disk_bytes = std::fs::read(&vault_path).unwrap();
        let disk_str = String::from_utf8_lossy(&raw_disk_bytes);
        assert!(!disk_str.contains("sk-proj-1234567890"));
        assert!(!disk_str.contains("openai_api_key"));

        // 2. Re-open vault from disk
        {
            let mut vault = Vault::open(&vault_path).unwrap();
            assert_eq!(vault.get("openai_api_key"), Some("sk-proj-1234567890"));
            assert_eq!(vault.get("anthropic_key"), Some("sk-ant-9876543210"));
            assert_eq!(vault.get("non_existent"), None);

            // Remove a key
            assert!(vault.remove("anthropic_key").unwrap());
            assert_eq!(vault.get("anthropic_key"), None);
            assert_eq!(vault.list_keys(), vec!["openai_api_key"]);
        }

        // 3. Re-open again to verify removal persisted
        {
            let vault = Vault::open(&vault_path).unwrap();
            assert_eq!(vault.get("openai_api_key"), Some("sk-proj-1234567890"));
            assert_eq!(vault.get("anthropic_key"), None);
            assert_eq!(vault.list_keys(), vec!["openai_api_key"]);
        }
    }
}
