use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A capability that an agent action may exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ReadFile,
    SearchFiles,
    WriteFile,
    ExecuteCommand,
    GitRead,
    GitWrite,
    NetworkRequest,
    AccessSecret,
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::ReadFile => "read files",
            Self::SearchFiles => "search files",
            Self::WriteFile => "write files",
            Self::ExecuteCommand => "execute commands",
            Self::GitRead => "inspect Git",
            Self::GitWrite => "modify Git",
            Self::NetworkRequest => "access the network",
            Self::AccessSecret => "access a secret",
        };
        formatter.write_str(name)
    }
}

/// User-facing severity assigned to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// How policy handles a capability before an action is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRule {
    Allow,
    Ask,
    Deny,
}

/// A complete, reviewable request shown before a protected action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub capability: Capability,
    pub risk: RiskLevel,
    pub summary: String,
    pub resource: String,
    pub details: Vec<String>,
}

/// A response supplied by a user-facing approval surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowForSession,
    AllowForProject,
    Deny,
}

/// Callback implemented by the CLI, desktop UI, or another host.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &PermissionRequest) -> PermissionDecision;
}

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("permission denied for {capability}: {summary}")]
    Denied {
        capability: Capability,
        summary: String,
    },
    #[error("approval is required for {capability}: {summary}")]
    ApprovalRequired {
        capability: Capability,
        summary: String,
    },
    #[error("permission state is unavailable")]
    StateUnavailable,
    #[error("Kernex could not determine a local configuration directory")]
    MissingConfigDirectory,
    #[error("could not access permission grants {path}: {source}")]
    GrantIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("permission grants are invalid: {0}")]
    GrantParse(#[from] toml::de::Error),
    #[error("permission grants could not be encoded: {0}")]
    GrantEncode(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    Ask,
    #[default]
    AutoSafe,
    FullAccess,
}

/// Policy defaults favor transparent reads and explicit approval for mutations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPolicy {
    #[serde(default)]
    mode: PermissionMode,
    rules: BTreeMap<Capability, PermissionRule>,
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        let mut rules = BTreeMap::new();
        rules.insert(Capability::ReadFile, PermissionRule::Allow);
        rules.insert(Capability::SearchFiles, PermissionRule::Allow);
        rules.insert(Capability::GitRead, PermissionRule::Allow);
        rules.insert(Capability::WriteFile, PermissionRule::Ask);
        rules.insert(Capability::ExecuteCommand, PermissionRule::Ask);
        rules.insert(Capability::GitWrite, PermissionRule::Ask);
        rules.insert(Capability::NetworkRequest, PermissionRule::Ask);
        rules.insert(Capability::AccessSecret, PermissionRule::Ask);
        Self {
            mode: PermissionMode::AutoSafe,
            rules,
        }
    }
}

impl PermissionPolicy {
    pub fn for_mode(mode: PermissionMode) -> Self {
        let mut policy = Self {
            mode,
            ..Self::default()
        };
        match mode {
            PermissionMode::ReadOnly => {
                for capability in [
                    Capability::WriteFile,
                    Capability::ExecuteCommand,
                    Capability::GitWrite,
                    Capability::NetworkRequest,
                    Capability::AccessSecret,
                ] {
                    policy.set(capability, PermissionRule::Deny);
                }
            }
            PermissionMode::Ask | PermissionMode::AutoSafe => {}
            PermissionMode::FullAccess => {
                for capability in [
                    Capability::ReadFile,
                    Capability::SearchFiles,
                    Capability::WriteFile,
                    Capability::ExecuteCommand,
                    Capability::GitRead,
                    Capability::GitWrite,
                    Capability::NetworkRequest,
                    Capability::AccessSecret,
                ] {
                    policy.set(capability, PermissionRule::Allow);
                }
            }
        }
        policy
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn set(&mut self, capability: Capability, rule: PermissionRule) {
        self.rules.insert(capability, rule);
    }

    pub fn rule(&self, capability: Capability) -> PermissionRule {
        self.rules
            .get(&capability)
            .copied()
            .unwrap_or(PermissionRule::Deny)
    }

    fn rule_for(&self, request: &PermissionRequest) -> PermissionRule {
        if self.mode == PermissionMode::AutoSafe && request.risk == RiskLevel::Low {
            return match request.capability {
                Capability::AccessSecret
                | Capability::WriteFile
                | Capability::GitWrite
                | Capability::NetworkRequest => self.rule(request.capability),
                _ => PermissionRule::Allow,
            };
        }
        self.rule(request.capability)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct PersistedProjectGrants {
    projects: BTreeMap<String, BTreeSet<Capability>>,
}

/// Non-sensitive, user-approved capability grants stored outside repositories.
pub struct ProjectGrantStore {
    path: PathBuf,
    grants: Mutex<PersistedProjectGrants>,
}

impl ProjectGrantStore {
    pub fn open_default() -> Result<Self, PermissionError> {
        let directories = ProjectDirs::from("dev", "Kernex", "Kernex")
            .ok_or(PermissionError::MissingConfigDirectory)?;
        Self::open(directories.config_dir().join("permissions.toml"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, PermissionError> {
        let path = path.as_ref().to_path_buf();
        let grants = if path.exists() {
            let contents =
                fs::read_to_string(&path).map_err(|source| PermissionError::GrantIo {
                    path: path.clone(),
                    source,
                })?;
            toml::from_str(&contents)?
        } else {
            PersistedProjectGrants::default()
        };
        Ok(Self {
            path,
            grants: Mutex::new(grants),
        })
    }

    fn allows(&self, project: &str, capability: Capability) -> Result<bool, PermissionError> {
        Ok(self
            .grants
            .lock()
            .map_err(|_| PermissionError::StateUnavailable)?
            .projects
            .get(project)
            .is_some_and(|grants| grants.contains(&capability)))
    }

    fn grant(&self, project: &str, capability: Capability) -> Result<(), PermissionError> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| PermissionError::StateUnavailable)?;
        grants
            .projects
            .entry(project.to_owned())
            .or_default()
            .insert(capability);
        let Some(parent) = self.path.parent() else {
            return Err(PermissionError::MissingConfigDirectory);
        };
        fs::create_dir_all(parent).map_err(|source| PermissionError::GrantIo {
            path: parent.to_path_buf(),
            source,
        })?;
        let encoded = toml::to_string_pretty(&*grants)?;
        fs::write(&self.path, encoded).map_err(|source| PermissionError::GrantIo {
            path: self.path.clone(),
            source,
        })
    }
}

/// Thread-safe policy evaluator shared by every tool in a session.
pub struct PermissionGate {
    policy: PermissionPolicy,
    approver: Option<Arc<dyn Approver>>,
    session_grants: Mutex<BTreeSet<(Capability, String)>>,
    project_scope: Option<String>,
    project_grants: Option<Arc<ProjectGrantStore>>,
}

impl PermissionGate {
    pub fn new(policy: PermissionPolicy, approver: Option<Arc<dyn Approver>>) -> Self {
        Self {
            policy,
            approver,
            session_grants: Mutex::new(BTreeSet::new()),
            project_scope: None,
            project_grants: None,
        }
    }

    pub fn for_project(
        policy: PermissionPolicy,
        approver: Option<Arc<dyn Approver>>,
        project_scope: impl Into<String>,
        project_grants: Arc<ProjectGrantStore>,
    ) -> Self {
        Self {
            policy,
            approver,
            session_grants: Mutex::new(BTreeSet::new()),
            project_scope: Some(project_scope.into()),
            project_grants: Some(project_grants),
        }
    }

    pub fn authorize(&self, request: &PermissionRequest) -> Result<(), PermissionError> {
        if self
            .session_grants
            .lock()
            .map_err(|_| PermissionError::StateUnavailable)?
            .contains(&(request.capability, request.resource.clone()))
        {
            return Ok(());
        }
        if let (Some(project), Some(grants)) = (&self.project_scope, &self.project_grants)
            && grants.allows(project, request.capability)?
        {
            return Ok(());
        }

        match self.policy.rule_for(request) {
            PermissionRule::Allow => Ok(()),
            PermissionRule::Deny => Err(PermissionError::Denied {
                capability: request.capability,
                summary: request.summary.clone(),
            }),
            PermissionRule::Ask => {
                let Some(approver) = &self.approver else {
                    return Err(PermissionError::ApprovalRequired {
                        capability: request.capability,
                        summary: request.summary.clone(),
                    });
                };
                match approver.decide(request) {
                    PermissionDecision::AllowOnce => Ok(()),
                    PermissionDecision::AllowForSession => {
                        self.session_grants
                            .lock()
                            .map_err(|_| PermissionError::StateUnavailable)?
                            .insert((request.capability, request.resource.clone()));
                        Ok(())
                    }
                    PermissionDecision::AllowForProject => {
                        let (Some(project), Some(grants)) =
                            (&self.project_scope, &self.project_grants)
                        else {
                            return Err(PermissionError::StateUnavailable);
                        };
                        grants.grant(project, request.capability)
                    }
                    PermissionDecision::Deny => Err(PermissionError::Denied {
                        capability: request.capability,
                        summary: request.summary.clone(),
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Always(PermissionDecision);

    impl Approver for Always {
        fn decide(&self, _request: &PermissionRequest) -> PermissionDecision {
            self.0
        }
    }

    fn write_request() -> PermissionRequest {
        PermissionRequest {
            capability: Capability::WriteFile,
            risk: RiskLevel::Medium,
            summary: "write src/main.rs".into(),
            resource: "src/main.rs".into(),
            details: Vec::new(),
        }
    }

    #[test]
    fn mutations_require_an_approver_by_default() {
        let gate = PermissionGate::new(PermissionPolicy::default(), None);
        assert!(matches!(
            gate.authorize(&write_request()),
            Err(PermissionError::ApprovalRequired { .. })
        ));
    }

    #[test]
    fn session_grant_skips_later_prompts() {
        let gate = PermissionGate::new(
            PermissionPolicy::default(),
            Some(Arc::new(Always(PermissionDecision::AllowForSession))),
        );
        gate.authorize(&write_request()).unwrap();
        gate.authorize(&write_request()).unwrap();
    }

    struct CountingApprover(AtomicUsize);

    impl Approver for CountingApprover {
        fn decide(&self, _request: &PermissionRequest) -> PermissionDecision {
            self.0.fetch_add(1, Ordering::Relaxed);
            PermissionDecision::AllowForSession
        }
    }

    #[test]
    fn session_grants_are_scoped_to_the_resource() {
        let approver = Arc::new(CountingApprover(AtomicUsize::new(0)));
        let gate = PermissionGate::new(PermissionPolicy::default(), Some(approver.clone()));
        let first = write_request();
        let mut second = write_request();
        second.resource = "src/lib.rs".into();
        gate.authorize(&first).unwrap();
        gate.authorize(&first).unwrap();
        gate.authorize(&second).unwrap();
        assert_eq!(approver.0.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn read_only_mode_rejects_writes_without_prompting() {
        let gate = PermissionGate::new(
            PermissionPolicy::for_mode(PermissionMode::ReadOnly),
            Some(Arc::new(Always(PermissionDecision::AllowOnce))),
        );
        assert!(matches!(
            gate.authorize(&write_request()),
            Err(PermissionError::Denied { .. })
        ));
    }

    #[test]
    fn auto_safe_mode_allows_low_risk_commands() {
        let gate = PermissionGate::new(PermissionPolicy::for_mode(PermissionMode::AutoSafe), None);
        gate.authorize(&PermissionRequest {
            capability: Capability::ExecuteCommand,
            risk: RiskLevel::Low,
            summary: "run a read-only command".into(),
            resource: ".".into(),
            details: Vec::new(),
        })
        .unwrap();
    }

    #[test]
    fn project_grants_persist_across_permission_gates() {
        let path: PathBuf = std::env::temp_dir().join(format!(
            "kernex-permissions-{}-{}.toml",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = Arc::new(ProjectGrantStore::open(&path).unwrap());
        let gate = PermissionGate::for_project(
            PermissionPolicy::default(),
            Some(Arc::new(Always(PermissionDecision::AllowForProject))),
            "/workspace",
            store,
        );
        gate.authorize(&write_request()).unwrap();

        let restored = Arc::new(ProjectGrantStore::open(&path).unwrap());
        let gate =
            PermissionGate::for_project(PermissionPolicy::default(), None, "/workspace", restored);
        let mut another_write = write_request();
        another_write.resource = "src/lib.rs".into();
        gate.authorize(&another_write).unwrap();

        fs::remove_file(path).unwrap();
    }
}
