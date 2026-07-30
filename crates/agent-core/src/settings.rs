use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::permission::PermissionMode;
use crate::provider::{ProviderConfig, ProviderKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSettings {
    pub name: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub auth_profile: Option<String>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            name: ProviderKind::OpenAiCompatible,
            model: String::new(),
            base_url: None,
            auth_profile: None,
        }
    }
}

impl ProviderSettings {
    pub fn to_provider_config(&self) -> ProviderConfig {
        let mut config = ProviderConfig::for_kind(self.name, self.model.clone());
        if let Some(base_url) = &self.base_url {
            config.base_url.clone_from(base_url);
        }
        config.auth_profile.clone_from(&self.auth_profile);
        config
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KernexSettings {
    pub provider: ProviderSettings,
    pub permission_mode: PermissionMode,
    pub recent_projects: Vec<String>,
    pub theme: String,
}

impl Default for KernexSettings {
    fn default() -> Self {
        Self {
            provider: ProviderSettings::default(),
            permission_mode: PermissionMode::AutoSafe,
            recent_projects: Vec::new(),
            theme: "dark".into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Kernex could not determine a local configuration directory")]
    MissingConfigDirectory,
    #[error("could not access settings {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("settings are invalid: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("settings could not be encoded: {0}")]
    Encode(#[from] toml::ser::Error),
}

impl KernexSettings {
    pub fn default_path() -> Result<PathBuf, SettingsError> {
        let directories = ProjectDirs::from("dev", "Kernex", "Kernex")
            .ok_or(SettingsError::MissingConfigDirectory)?;
        Ok(directories.config_dir().join("config.toml"))
    }

    pub fn load_default() -> Result<Self, SettingsError> {
        Self::load(Self::default_path()?)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path).map_err(|source| SettingsError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(toml::from_str(&contents)?)
    }

    pub fn save_default(&self) -> Result<(), SettingsError> {
        self.save(Self::default_path()?)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SettingsError> {
        let path = path.as_ref();
        let Some(parent) = path.parent() else {
            return Err(SettingsError::MissingConfigDirectory);
        };
        fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        fs::write(path, toml::to_string_pretty(self)?).map_err(|source| SettingsError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn remember_project(&mut self, path: impl Into<String>) {
        let path = path.into();
        self.recent_projects.retain(|existing| existing != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_without_credentials() {
        let path = std::env::temp_dir().join(format!(
            "kernex-settings-{}-{}.toml",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let mut settings = KernexSettings::default();
        settings.provider.model = "test-model".into();
        settings.provider.auth_profile = Some("personal".into());
        settings.remember_project("/workspace");
        settings.save(&path).unwrap();

        let restored = KernexSettings::load(&path).unwrap();
        assert_eq!(restored.provider.model, "test-model");
        assert_eq!(restored.provider.auth_profile.as_deref(), Some("personal"));
        assert_eq!(restored.recent_projects, ["/workspace"]);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn desktop_theme_defaults_to_dark() {
        assert_eq!(KernexSettings::default().theme, "dark");
    }
}
