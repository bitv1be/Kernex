use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::command::{CommandError, CommandRunner, CommandSpec};
use crate::permission::{Capability, RiskLevel};
use crate::provider::ToolDefinition;
use crate::workspace::{Workspace, WorkspaceError};

#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub path: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone)]
pub struct PluginTool {
    pub qualified_name: String,
    pub definition: ToolDefinition,
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: Vec<PluginInfo>,
    tools: Vec<PluginTool>,
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("could not inspect plugin directory {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse plugin manifest {path}: {source}")]
    Manifest {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid plugin identifier `{0}`; use letters, digits, `_`, or `-`")]
    InvalidIdentifier(String),
    #[error("plugin tool {0} has an empty command")]
    EmptyCommand(String),
    #[error("duplicate plugin tool name: {0}")]
    DuplicateTool(String),
    #[error("plugin tool {tool} has invalid input_schema JSON: {source}")]
    InvalidSchema {
        tool: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("plugin tool {tool} failed: {stderr}")]
    Execution { tool: String, stderr: String },
}

#[derive(Debug, Deserialize)]
struct PluginManifest {
    plugin: PluginMetadata,
    #[serde(default)]
    tools: Vec<PluginToolManifest>,
}

#[derive(Debug, Deserialize)]
struct PluginMetadata {
    id: String,
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct PluginToolManifest {
    name: String,
    description: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    input_schema: Option<String>,
}

impl PluginRegistry {
    /// Loads declarative process plugins from `.kernex/plugins/*/plugin.toml`.
    pub fn discover(workspace: &Workspace) -> Result<Self, PluginError> {
        let plugin_root = workspace.root().join(".kernex/plugins");
        if !plugin_root.exists() {
            return Ok(Self::default());
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(&plugin_root).map_err(|source| PluginError::Io {
            path: plugin_root.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| PluginError::Io {
                path: plugin_root.display().to_string(),
                source,
            })?;
            let manifest = entry.path().join("plugin.toml");
            if manifest.is_file() {
                manifests.push(manifest);
            }
        }
        manifests.sort();

        let mut registry = Self::default();
        for manifest_path in manifests {
            let resolved = workspace.resolve_existing(&manifest_path)?;
            let manifest_text = workspace.read_text(&resolved)?;
            let manifest: PluginManifest =
                toml::from_str(&manifest_text).map_err(|source| PluginError::Manifest {
                    path: workspace.display_path(&resolved),
                    source,
                })?;
            validate_identifier(&manifest.plugin.id)?;
            let plugin_dir = resolved.parent().expect("plugin manifest has a parent");
            let relative_dir = PathBuf::from(workspace.display_path(plugin_dir));
            let tool_count = manifest.tools.len();
            for tool in manifest.tools {
                validate_identifier(&tool.name)?;
                let qualified_name = format!("plugin__{}__{}", manifest.plugin.id, tool.name);
                if registry
                    .tools
                    .iter()
                    .any(|existing| existing.qualified_name == qualified_name)
                {
                    return Err(PluginError::DuplicateTool(qualified_name));
                }
                if tool.command.trim().is_empty() {
                    return Err(PluginError::EmptyCommand(qualified_name));
                }
                let input_schema = match tool.input_schema {
                    Some(schema) => serde_json::from_str(&schema).map_err(|source| {
                        PluginError::InvalidSchema {
                            tool: qualified_name.clone(),
                            source,
                        }
                    })?,
                    None => json!({"type": "object"}),
                };
                registry.tools.push(PluginTool {
                    definition: ToolDefinition {
                        name: qualified_name.clone(),
                        description: tool.description,
                        input_schema,
                        output_schema: json!({"type": ["object", "array", "string"]}),
                        risk_level: RiskLevel::High,
                        permission: Capability::ExecuteCommand,
                        supports_cancellation: true,
                        timeout_seconds: Some(120),
                    },
                    qualified_name,
                    program: tool.command,
                    args: tool.args,
                    cwd: relative_dir.clone(),
                });
            }
            registry.plugins.push(PluginInfo {
                id: manifest.plugin.id,
                name: manifest.plugin.name,
                version: manifest.plugin.version,
                path: workspace.display_path(plugin_dir),
                tool_count,
            });
        }
        Ok(registry)
    }

    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    pub fn definitions(&self) -> impl Iterator<Item = ToolDefinition> + '_ {
        self.tools.iter().map(|tool| tool.definition.clone())
    }

    pub fn find(&self, qualified_name: &str) -> Option<&PluginTool> {
        self.tools
            .iter()
            .find(|tool| tool.qualified_name == qualified_name)
    }
}

impl PluginTool {
    pub async fn execute(
        &self,
        runner: &CommandRunner,
        arguments: &Value,
    ) -> Result<String, PluginError> {
        let mut command = CommandSpec::new(&self.program, &self.args);
        command.cwd = Some(self.cwd.clone());
        command.stdin = Some(format!("{}\n", arguments));
        let output = runner.run(&command).await?;
        if !output.success {
            return Err(PluginError::Execution {
                tool: self.qualified_name.clone(),
                stderr: output.stderr,
            });
        }
        Ok(output.stdout)
    }
}

fn validate_identifier(identifier: &str) -> Result<(), PluginError> {
    if !identifier.is_empty()
        && identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        Ok(())
    } else {
        Err(PluginError::InvalidIdentifier(identifier.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_plugin_identifiers() {
        assert!(validate_identifier("git-tools_2").is_ok());
        assert!(validate_identifier("../escape").is_err());
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn empty_registry_when_plugin_directory_is_absent() {
        let workspace = Workspace::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let registry = PluginRegistry::discover(&workspace).unwrap();
        assert!(registry.plugins().is_empty());
    }
}
