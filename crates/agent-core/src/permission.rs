use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub capability: Capability,
    pub risk: RiskLevel,
    pub summary: String,
    pub resource: String,
    pub details: Vec<String>,
}

/// A response supplied by a user-facing approval surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowForSession,
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
}

/// Policy defaults favor transparent reads and explicit approval for mutations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPolicy {
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
        Self { rules }
    }
}

impl PermissionPolicy {
    pub fn set(&mut self, capability: Capability, rule: PermissionRule) {
        self.rules.insert(capability, rule);
    }

    pub fn rule(&self, capability: Capability) -> PermissionRule {
        self.rules
            .get(&capability)
            .copied()
            .unwrap_or(PermissionRule::Deny)
    }
}

/// Thread-safe policy evaluator shared by every tool in a session.
pub struct PermissionGate {
    policy: PermissionPolicy,
    approver: Option<Arc<dyn Approver>>,
    session_grants: Mutex<BTreeSet<(Capability, String)>>,
}

impl PermissionGate {
    pub fn new(policy: PermissionPolicy, approver: Option<Arc<dyn Approver>>) -> Self {
        Self {
            policy,
            approver,
            session_grants: Mutex::new(BTreeSet::new()),
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

        match self.policy.rule(request.capability) {
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
}
