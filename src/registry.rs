use crate::providers::{openrouter_client, ChatAgent};
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Orchestrator,
    Planner,
    Builder,
    Auditor,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RoleConfig {
    pub model: String,
    pub preamble: String,
    #[serde(default)]
    pub permissions: Permissions,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Permissions {
    #[serde(default)]
    pub edit: Permission,
    #[serde(default)]
    pub bash: Permission,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    #[default]
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Deserialize)]
pub struct AgentRegistryConfig {
    #[serde(rename = "agents")]
    pub roles: std::collections::HashMap<String, RoleConfig>,
}

impl AgentRegistryConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: AgentRegistryConfig = toml::from_str(&raw)?;
        Ok(cfg)
    }
}

/// A role-bound agent: model + preamble (loaded from a .md file) + permissions.
pub struct RoleAgent {
    #[allow(dead_code)]
    pub role: Role,
    agent: ChatAgent,
    // Permissions are enforced at build time (which tools an agent gets). The
    // fields are retained for runtime inspection / Phase-3 policy checks.
    #[allow(dead_code)]
    pub permissions: Permissions,
}

impl RoleAgent {
    pub async fn run(&self, task: &str) -> anyhow::Result<String> {
        Ok(self.agent.prompt(task).await?)
    }
}

pub struct AgentRegistry {
    config: Arc<AgentRegistryConfig>,
}

impl AgentRegistry {
    pub fn new(config: AgentRegistryConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
        }
    }

    pub fn build(&self, role: Role) -> anyhow::Result<RoleAgent> {
        let key = format!("{role:?}").to_lowercase();
        let rc = self
            .config
            .roles
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("no config for role {key}"))?;
        let client = openrouter_client()?;
        let preamble = std::fs::read_to_string(&rc.preamble)
            .unwrap_or_else(|_| format!("You are the {key} agent."));
        let with_tools =
            rc.permissions.edit == Permission::Allow || rc.permissions.bash == Permission::Allow;
        let agent = if with_tools {
            let tools = crate::tools::builtin_tools()?;
            client
                .agent(&rc.model)
                .preamble(&preamble)
                .temperature(0.7)
                .tools(tools)
                .build()
        } else {
            client
                .agent(&rc.model)
                .preamble(&preamble)
                .temperature(0.7)
                .build()
        };
        Ok(RoleAgent {
            role,
            agent,
            permissions: rc.permissions.clone(),
        })
    }
}

/// Classify a user message into an intent, mirroring OMO's Intent Gate.
#[derive(Debug, PartialEq, Eq)]
pub enum Intent {
    Implement,
    Investigate,
    Chat,
}

pub fn classify(message: &str) -> Intent {
    let m = message.to_lowercase();
    if ["implement", "add", "create", "fix", "write", "build"]
        .iter()
        .any(|k| m.contains(k))
    {
        Intent::Implement
    } else if ["look into", "investigate", "check", "find", "how does"]
        .iter()
        .any(|k| m.contains(k))
    {
        Intent::Investigate
    } else {
        Intent::Chat
    }
}

/// Orchestrator: classify intent, then delegate to the right role-agent. This is
/// the Sisyphus-equivalent. For Phase 2 it does direct delegation; Phase 3 adds
/// the planner/review subagent loop.
pub struct Orchestrator {
    registry: AgentRegistry,
}

impl Orchestrator {
    pub fn new(registry: AgentRegistry) -> Self {
        Self { registry }
    }

    pub async fn handle(&self, message: &str) -> anyhow::Result<String> {
        let intent = classify(message);
        let role = match intent {
            Intent::Implement => Role::Builder,
            Intent::Investigate => Role::Planner,
            Intent::Chat => Role::Orchestrator,
        };
        let agent = self.registry.build(role)?;
        agent.run(message).await
    }
}
