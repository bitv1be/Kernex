use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::lsp::LanguageServerConfig;
use crate::mcp::McpServerConfig;
use crate::settings::ProviderSettings;
use crate::workspace::{Workspace, WorkspaceError};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KernexConfig {
    pub provider: Option<ProviderSettings>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub language_servers: Vec<LanguageServerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageServerEntry {
    #[serde(flatten)]
    pub server: LanguageServerConfig,
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("could not parse .kernex/config.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

impl KernexConfig {
    pub fn parse(contents: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(contents)?)
    }

    pub fn load(workspace: &Workspace) -> Result<Self, ConfigError> {
        let path = workspace.root().join(".kernex/config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = workspace.read_text(path)?;
        Self::parse(&contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extension_configuration() {
        let config: KernexConfig = toml::from_str(
            r#"
                [[mcp_servers]]
                name = "filesystem"
                command = "example-mcp"
                args = ["--stdio"]

                [[language_servers]]
                language_id = "rust"
                command = "rust-analyzer"
                extensions = ["rs"]
            "#,
        )
        .unwrap();
        assert_eq!(config.mcp_servers[0].name, "filesystem");
        assert_eq!(config.language_servers[0].extensions, ["rs"]);
    }
}
