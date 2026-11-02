//! Extension manifest schema, validation, and host loader.
//!
//! Parses `extension.json` manifests, validates capability permissions,
//! prevents directory traversal (Zip Slip), and discovers installed
//! extensions in the local AutoScene extensions directory.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// Result type for extension operations.
pub type ExtensionResult<T> = Result<T, ExtensionError>;

/// Errors encountered while loading or validating extensions.
#[derive(Debug, Error)]
pub enum ExtensionError {
    #[error("Manifest file not found: {0}")]
    NotFound(String),
    #[error("Invalid extension manifest: {0}")]
    InvalidManifest(String),
    #[error("Insecure path traversal detected: {0}")]
    InsecurePath(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Declared capability permissions an extension can request.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionCapability {
    FilesystemReadProject,
    FilesystemWriteExport,
    NetworkWebhook,
    Gpu,
}

/// Extension type and lifecycle target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionKind {
    Exporter,
    PostProcessor,
    Webhook,
    Custom,
}

/// The `extension.json` manifest structure.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub kind: ExtensionKind,
    pub entrypoint: String,
    pub capabilities: Vec<ExtensionCapability>,
    #[serde(default)]
    pub triggers: Vec<String>,
}

impl ExtensionManifest {
    /// Validates basic security boundaries and manifest sanity.
    pub fn validate(&self) -> ExtensionResult<()> {
        if self.id.trim().is_empty() {
            return Err(ExtensionError::InvalidManifest("id cannot be empty".into()));
        }
        if self.entrypoint.contains("..") || self.entrypoint.starts_with('/') || self.entrypoint.starts_with('\\') {
            return Err(ExtensionError::InsecurePath(format!(
                "entrypoint '{}' contains invalid traversal or absolute path",
                self.entrypoint
            )));
        }
        Ok(())
    }
}

/// Discovered extension installed in the system.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtension {
    pub manifest: ExtensionManifest,
    pub path: String,
    pub entrypoint_path: String,
}

/// Scans a directory for installed extensions containing `extension.json`.
pub fn discover_extensions(extensions_dir: &Path) -> Vec<InstalledExtension> {
    let mut installed = Vec::new();
    let Ok(entries) = fs::read_dir(extensions_dir) else {
        return installed;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("extension.json");
        if !manifest_path.is_file() {
            continue;
        }

        let Ok(data) = fs::read(&manifest_path) else {
            continue;
        };

        let Ok(manifest) = serde_json::from_slice::<ExtensionManifest>(&data) else {
            continue;
        };

        if manifest.validate().is_err() {
            continue;
        }

        let entrypoint_path = path.join(&manifest.entrypoint);
        installed.push(InstalledExtension {
            path: path.to_string_lossy().to_string(),
            entrypoint_path: entrypoint_path.to_string_lossy().to_string(),
            manifest,
        });
    }

    installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_and_deserializes_manifest() {
        let manifest = ExtensionManifest {
            id: "my-ext".into(),
            name: "My Ext".into(),
            version: "0.1.0".into(),
            description: "Test".into(),
            author: "Dev".into(),
            kind: ExtensionKind::Exporter,
            entrypoint: "main.py".into(),
            capabilities: vec![ExtensionCapability::FilesystemWriteExport],
            triggers: vec![],
        };
        let serialized = serde_json::to_string(&manifest).unwrap();
        let parsed = serde_json::from_str::<ExtensionManifest>(&serialized).unwrap();
        assert_eq!(parsed.id, "my-ext");
        assert_eq!(parsed.kind, ExtensionKind::Exporter);
        assert_eq!(parsed.capabilities, vec![ExtensionCapability::FilesystemWriteExport]);
    }

    #[test]
    fn validates_clean_manifest() {
        let manifest = ExtensionManifest {
            id: "com.example.discord-notify".into(),
            name: "Discord Notifier".into(),
            version: "1.0.0".into(),
            description: "Sends notifications".into(),
            author: "Tester".into(),
            kind: ExtensionKind::Webhook,
            entrypoint: "notify.sh".into(),
            capabilities: vec![ExtensionCapability::NetworkWebhook],
            triggers: vec!["post-export".into()],
        };

        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn rejects_path_traversal_in_entrypoint() {
        let manifest = ExtensionManifest {
            id: "com.example.malicious".into(),
            name: "Malicious".into(),
            version: "1.0.0".into(),
            description: "Bad".into(),
            author: "Attacker".into(),
            kind: ExtensionKind::Custom,
            entrypoint: "../../etc/passwd".into(),
            capabilities: vec![],
            triggers: vec![],
        };

        assert!(manifest.validate().is_err());
    }

    #[test]
    fn discovers_valid_extensions_in_folder() {
        let temp = tempfile::tempdir().unwrap();
        let ext_dir = temp.path().join("my-ext");
        fs::create_dir_all(&ext_dir).unwrap();

        let manifest = ExtensionManifest {
            id: "my-ext".into(),
            name: "My Ext".into(),
            version: "0.1.0".into(),
            description: "Test".into(),
            author: "Dev".into(),
            kind: ExtensionKind::Exporter,
            entrypoint: "main.py".into(),
            capabilities: vec![ExtensionCapability::FilesystemWriteExport],
            triggers: vec![],
        };

        let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        fs::write(ext_dir.join("extension.json"), manifest_bytes).unwrap();
        fs::write(ext_dir.join("main.py"), "print('hello')").unwrap();

        let found = discover_extensions(temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.id, "my-ext");
        assert_eq!(found[0].manifest.kind, ExtensionKind::Exporter);
    }
}
