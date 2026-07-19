use crate::providers::{openrouter_client, ChatAgent};
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

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
    pub permissions: ToolPerms,
}

// Per-tool permission tiers for the autonomous loop's human-in-the-loop gate.
// `allow` = auto-run without prompting; `ask` = pause for human confirmation;
// `deny` = block the call and explain to the model.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct ToolPerms {
    #[serde(default = "default_allow")]
    pub read_file: Permission,
    #[serde(default = "default_allow")]
    pub run_bash_readonly: Permission,
    #[serde(default = "default_ask")]
    pub run_bash_mutating: Permission,
    #[serde(default = "default_ask")]
    pub edit_file: Permission,
}

fn default_allow() -> Permission {
    Permission::Allow
}
fn default_ask() -> Permission {
    Permission::Ask
}

impl Default for ToolPerms {
    fn default() -> Self {
        ToolPerms {
            read_file: Permission::Allow,
            run_bash_readonly: Permission::Allow,
            run_bash_mutating: Permission::Ask,
            edit_file: Permission::Ask,
        }
    }
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
    #[serde(default)]
    pub max_turns: usize,
    #[serde(rename = "agents")]
    pub roles: std::collections::HashMap<String, RoleConfig>,
}

impl AgentRegistryConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: AgentRegistryConfig = toml::from_str(&raw)?;
        Ok(cfg)
    }

    pub fn max_turns(&self) -> usize {
        if self.max_turns == 0 {
            20
        } else {
            self.max_turns
        }
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
    pub permissions: ToolPerms,
}

impl RoleAgent {
    pub async fn run(&self, task: &str) -> anyhow::Result<String> {
        Ok(self.agent.prompt(task).await?)
    }
}

pub struct AgentRegistry {
    config: Arc<AgentRegistryConfig>,
    // Runtime model override for the whole session. When set, every role uses
    // this slug instead of its configured model. Lets the user switch to a
    // free model (e.g. tencent/hy3:free) from the REPL without editing files.
    session_model: Arc<Mutex<Option<String>>>,
}

impl AgentRegistry {
    pub fn new(config: AgentRegistryConfig) -> Self {
        Self {
            config: Arc::new(config),
            session_model: Arc::new(Mutex::new(None)),
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            session_model: self.session_model.clone(),
        }
    }

    /// Override the model used by all roles for this session.
    pub fn set_session_model(&self, slug: &str) {
        *self.session_model.lock().unwrap() = Some(slug.to_string());
    }

    pub fn session_model(&self) -> Option<String> {
        self.session_model.lock().unwrap().clone()
    }

    /// Loop cap for the autonomous agent run, forwarded from config.
    pub fn max_turns(&self) -> usize {
        self.config.max_turns()
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
        // Session override wins over the per-role configured model.
        let model = match *self.session_model.lock().unwrap() {
            Some(ref m) => m.clone(),
            None => rc.model.clone(),
        };
        // Grant tools whenever any builtin tool is allowed for this role.
        let with_tools = rc.permissions.read_file == Permission::Allow
            || rc.permissions.run_bash_readonly == Permission::Allow
            || rc.permissions.run_bash_mutating == Permission::Allow
            || rc.permissions.edit_file == Permission::Allow;
        let agent = if with_tools {
            let tools = crate::tools::builtin_tools()?;
            client
                .agent(&model)
                .preamble(&preamble)
                .temperature(0.7)
                .tools(tools)
                .build()
        } else {
            client
                .agent(&model)
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

    /// Per-tool permission tiers for a role, used by the autonomous loop's
    /// human-in-the-loop gate to decide allow / ask / deny per tool call.
    pub fn tool_perms(&self, role: Role) -> ToolPerms {
        let key = format!("{role:?}").to_lowercase();
        self.config
            .roles
            .get(&key)
            .map(|rc| rc.permissions.clone())
            .unwrap_or_default()
    }

    /// Role config lookup used by the autonomous loop to rebuild a runner-capable
    /// agent (the loop needs the raw `Agent`, not the `RoleAgent` wrapper).
    pub fn role_config(&self, role: Role) -> Option<&RoleConfig> {
        let key = format!("{role:?}").to_lowercase();
        self.config.roles.get(&key)
    }
}

/// Classify a user message into an intent, mirroring OMO's Intent Gate.
/// Reserved for the one-shot `chat` mode (the autonomous loop currently handles
/// all non-meta input directly).
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub enum Intent {
    Implement,
    Investigate,
    Chat,
}

#[allow(dead_code)]
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
/// the Sisyphus-equivalent. Reserved for the one-shot `chat` mode; the autonomous
/// loop currently handles non-meta input directly.
#[allow(dead_code)]
pub struct Orchestrator {
    registry: AgentRegistry,
}

#[allow(dead_code)]
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
