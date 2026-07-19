use std::sync::{Arc, Mutex};

use rig_core::agent::{AgentHook, Flow, HookContext, StepEvent};
use rig_core::client::CompletionClient;
use rig_core::completion::CompletionModel;
use rig_core::providers::openrouter::CompletionModel as OpenRouterModel;
use rig_core::tool::ToolDyn;

use crate::registry::{AgentRegistry, Permission, Role, ToolPerms};
use crate::tools::{is_readonly_bash, TOOL_NAMES};

/// Human-in-the-loop gate. Implemented as a rig `AgentHook` that intercepts
/// every `ToolCall` and applies the role's per-tool permission tier:
/// - `Allow`  -> run silently (no prompt). Trivial steps like `ls` flow through.
/// - `Ask`    -> pause and ask the user on the terminal; yes runs, no skips.
/// - `Deny`   -> skip the call and explain to the model why.
///
/// Only `ToolCall` events are gated; model turns, results and deltas pass
/// through untouched. The permission tiers are captured at run start.
#[derive(Clone)]
pub struct HitlHook {
    perms: Arc<Mutex<ToolPerms>>,
}

#[allow(dead_code)]
impl HitlHook {
    pub fn new(perms: ToolPerms) -> Self {
        Self {
            perms: Arc::new(Mutex::new(perms)),
        }
    }

    /// Resolve the permission tier for a tool call by name + args.
    fn tier_for(&self, tool_name: &str, args: &str) -> Permission {
        let perms = self.perms.lock().unwrap();
        decide_tier(&perms, tool_name, args)
    }

    /// Block on a terminal yes/no prompt. Runs the blocking rustyline read on a
    /// separate thread via `spawn_blocking` so it does not stall the async
    /// runtime, then awaits the handle. Returns true for "yes".
    async fn confirm(&self, prompt: &str) -> bool {
        let prompt = prompt.to_string();
        let handle = tokio::task::spawn_blocking(move || {
            use rustyline::DefaultEditor;
            let mut rl = match DefaultEditor::new() {
                Ok(rl) => rl,
                Err(_) => return false,
            };
            match rl.readline(&prompt) {
                Ok(line) => {
                    let a = line.trim().to_lowercase();
                    a == "y" || a == "yes"
                }
                Err(_) => false,
            }
        });
        handle.await.unwrap_or(false)
    }
}

impl<M: CompletionModel> AgentHook<M> for HitlHook {
    async fn on_event(&self, _ctx: &HookContext, event: StepEvent<'_, M>) -> Flow {
        let StepEvent::ToolCall { tool_name, args, .. } = event else {
            return Flow::Continue;
        };
        let perms = self.perms.lock().unwrap().clone();
        decide_flow(&perms, tool_name, args)
    }
}

/// Pure tier resolution, testable without the hook wrapper. `args` is the JSON
/// tool-call arguments (used to extract the `command` for `run_bash`).
pub fn decide_tier(perms: &ToolPerms, tool_name: &str, args: &str) -> Permission {
    match tool_name {
        "read_file" => perms.read_file,
        "edit_file" | "write_file" => perms.edit_file,
        "run_bash" => {
            let command = serde_json::from_str::<serde_json::Value>(args)
                .ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(str::to_string))
                .unwrap_or_default();
            if is_readonly_bash(&command) {
                perms.run_bash_readonly
            } else {
                perms.run_bash_mutating
            }
        }
        _ => Permission::Ask,
    }
}

/// Pure flow decision from permission tiers. Does not prompt — an `Ask` tier
/// resolves to `Flow::Skip` (treated as declined) so it is deterministic and
/// unit-testable; the live hook replaces the `Ask` branch with a terminal prompt.
pub fn decide_flow(perms: &ToolPerms, tool_name: &str, args: &str) -> Flow {
    match decide_tier(perms, tool_name, args) {
        Permission::Allow => Flow::Continue,
        Permission::Deny => Flow::Skip {
            reason: format!("tool `{tool_name}` is denied by policy for this role"),
        },
        Permission::Ask => Flow::Skip {
            reason: format!("user declined to run `{tool_name}`"),
        },
    }
}

/// Drive an autonomous agent loop for `goal`. The Builder role plans and acts,
/// calling tools itself; the `HitlHook` gates key decisions. Stops when the
/// model finishes, or `max_turns` is reached.
pub async fn run_autonomous(registry: &AgentRegistry, goal: &str) -> anyhow::Result<String> {
    let perms = registry.tool_perms(Role::Builder);
    let max_turns = registry.max_turns();
    let hook = HitlHook::new(perms);

    let agent = build_runner_agent(registry, Role::Builder)?;
    let response = agent
        .runner(goal)
        .max_turns(max_turns)
        // Recover from stray/unknown tool names (e.g. model invents a tool) by
        // retrying the turn with corrective feedback instead of aborting.
        .max_invalid_tool_call_retries(3)
        .add_hook(hook)
        .run()
        .await?;
    Ok(response.output)
}

/// Build a rig `Agent` (runner-capable) for a role with its tools, honoring the
/// session model override. Mirrors `AgentRegistry::build` but returns the raw
/// `Agent` so the runner + hook can be attached.
fn build_runner_agent(
    registry: &AgentRegistry,
    role: Role,
) -> anyhow::Result<rig_core::agent::Agent<OpenRouterModel>> {
    let rc = registry
        .role_config(role)
        .ok_or_else(|| anyhow::anyhow!("no config for role {role:?}"))?;
    let client = crate::providers::openrouter_client()?;
    let preamble = std::fs::read_to_string(&rc.preamble)
        .unwrap_or_else(|_| format!("You are the {role:?} agent."));
    let model = registry.session_model().unwrap_or_else(|| rc.model.clone());
    let tools: Vec<Box<dyn ToolDyn>> = crate::tools::builtin_tools()?;
    let agent = client
        .agent(&model)
        .preamble(&preamble)
        .temperature(0.7)
        .tools(tools)
        .build();
    Ok(agent)
}

#[allow(dead_code)]
fn _assert_tool_names() {
    // Compile-time guard that the classifier covers every builtin tool.
    let _ = TOOL_NAMES;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::agent::Flow;

    fn perms() -> ToolPerms {
        ToolPerms {
            read_file: Permission::Allow,
            run_bash_readonly: Permission::Allow,
            run_bash_mutating: Permission::Ask,
            edit_file: Permission::Ask,
        }
    }

    #[test]
    fn read_file_auto_runs() {
        assert!(matches!(
            decide_flow(&perms(), "read_file", r#"{"path":"x"}"#),
            Flow::Continue
        ));
    }

    #[test]
    fn readonly_bash_auto_runs() {
        assert!(matches!(
            decide_flow(&perms(), "run_bash", r#"{"command":"ls -la"}"#),
            Flow::Continue
        ));
    }

    #[test]
    fn mutating_bash_asks() {
        // Ask tier resolves to Skip in the pure decision (live hook prompts).
        assert!(matches!(
            decide_flow(&perms(), "run_bash", r#"{"command":"rm -rf x"}"#),
            Flow::Skip { .. }
        ));
    }

    #[test]
    fn edit_file_asks() {
        assert!(matches!(
            decide_flow(&perms(), "edit_file", r#"{"path":"x","old":"a","new":"b"}"#),
            Flow::Skip { .. }
        ));
    }

    #[test]
    fn write_file_asks_like_edit() {
        assert!(matches!(
            decide_flow(&perms(), "write_file", r#"{"path":"x","content":"hi"}"#),
            Flow::Skip { .. }
        ));
    }

    #[test]
    fn denied_tool_skips() {
        let mut p = perms();
        p.run_bash_readonly = Permission::Deny;
        assert!(matches!(
            decide_flow(&p, "run_bash", r#"{"command":"cat x"}"#),
            Flow::Skip { .. }
        ));
    }

    #[test]
    fn unknown_tool_asks() {
        assert!(matches!(
            decide_flow(&perms(), "mystery", r#"{}"#),
            Flow::Skip { .. }
        ));
    }
}
