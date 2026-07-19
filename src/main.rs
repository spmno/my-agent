mod memory;
mod registry;
mod providers;
mod reviewer;
mod tools;
mod evolution;
mod agent_loop;

use anyhow::Result;
use evolution::{
    prompt_evolve::PromptEvolver,
    self_modify,
    tool_ext,
};
use registry::{AgentRegistryConfig, AgentRegistry, Orchestrator};
use reviewer::ReviewGate;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let reg_cfg = AgentRegistryConfig::load("agent.toml")?;
    #[allow(unused)]
    let threshold = load_escalation_threshold()?; // reserved for future review-gated mode
    let registry = AgentRegistry::new(reg_cfg);
    #[allow(unused)]
    let orchestrator = Orchestrator::new(registry.clone()); // reserved for one-shot `chat` mode
    #[allow(unused)]
    let reviewer = ReviewGate::new(registry.clone()); // reserved for review-gated mode
    let evolver = PromptEvolver::new(registry.clone(), "AGENTS.md".to_string());

    let mem = memory::MemoryStore::new(&load_memory_cfg()?)?;

    println!(
        "my-agent ready (OpenRouter). model: {}\nCommands: model <slug> | evolve | evolve-code | add-tool | quit",
        current_model(&registry)
    );

    // rustyline gives proper UTF-8 / IME line editing: Backspace removes one
    // char (not one byte), ↑/↓ history works, and CJK input composes correctly.
    let mut rl = rustyline::DefaultEditor::new()?;
    loop {
        let read = rl.readline("> ");
        let line = match read {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        rl.add_history_entry(line.as_str())?;
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "quit" {
            break;
        }

        // REPL meta-commands. Order matters: "evolve-code"/"add-tool" must be
        // checked before the "evolve" prefix, since "evolve-code" starts with
        // "evolve". "model" is a distinct command.
        if let Some(rest) = input.strip_prefix("model") {
            let slug = rest.trim();
            if slug.is_empty() {
                println!("current model: {}", current_model(&registry));
                continue;
            }
            registry.set_session_model(slug);
            println!("model set to: {slug} (applies to all roles this session)");
            continue;
        }
        if let Some(rest) = input.strip_prefix("evolve-code") {
            let rest = rest.trim();
            let parts: Vec<String> = rest
                .splitn(3, ' ')
                .map(|p| p.trim_matches('"').to_string())
                .collect();
            if parts.len() < 3 {
                println!("usage: evolve-code <file> <old_text> <new_text>");
                continue;
            }
            match self_modify::evolve_code(&parts[0], &parts[1], &parts[2]) {
                Ok(msg) => println!("{msg}"),
                Err(e) => eprintln!("evolve-code error: {e}"),
            }
            continue;
        }
        if let Some(rest) = input.strip_prefix("add-tool") {
            let rest = rest.trim();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() < 2 {
                println!("usage: add-tool <name> <description>");
                continue;
            }
            match tool_ext::add_tool(parts[0], parts[1]) {
                Ok(msg) => println!("{msg}"),
                Err(e) => eprintln!("add-tool error: {e}"),
            }
            continue;
        }
        if input.strip_prefix("evolve").is_some() {
            match evolver.evolve().await {
                Ok(msg) => println!("{msg}"),
                Err(e) => eprintln!("evolve error: {e}"),
            }
            continue;
        }

        // Any non-meta input is a goal for the autonomous agent loop. The loop
        // runs the Builder role, which plans and calls tools itself; the
        // human-in-the-loop hook pauses only for Ask-tier tool calls.
        match agent_loop::run_autonomous(&registry, input).await {
            Ok(out) => {
                println!("{out}");
                mem.append_turn(&memory::Turn {
                    role: "user".into(),
                    content: input.into(),
                    ts: now(),
                })?;
                mem.append_turn(&memory::Turn {
                    role: "agent".into(),
                    content: out,
                    ts: now(),
                })?;
            }
            Err(e) => eprintln!("autonomous loop error: {e}"),
        }
    }
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_memory_cfg() -> Result<memory::MemoryConfig> {
    let raw = std::fs::read_to_string("agent.toml")?;
    let parsed: toml::Value = toml::from_str(&raw)?;
    let m = parsed
        .get("memory")
        .ok_or_else(|| anyhow::anyhow!("missing [memory] in agent.toml"))?;
    let cfg: memory::MemoryConfig = m.clone().try_into()?;
    Ok(cfg)
}

fn load_escalation_threshold() -> Result<u32> {
    let raw = std::fs::read_to_string("agent.toml")?;
    let parsed: toml::Value = toml::from_str(&raw)?;
    let t = parsed
        .get("evolution")
        .and_then(|e| e.get("rule_escalation_threshold"))
        .and_then(|v| v.as_integer())
        .unwrap_or(3);
    Ok(t as u32)
}

fn current_model(registry: &registry::AgentRegistry) -> String {
    match registry.session_model() {
        Some(m) => m,
        None => load_default_model().unwrap_or_else(|| "deepseek/deepseek-chat".to_string()),
    }
}

fn load_default_model() -> Option<String> {
    let raw = std::fs::read_to_string("agent.toml").ok()?;
    let parsed: toml::Value = toml::from_str(&raw).ok()?;
    parsed
        .get("agent")
        .and_then(|a| a.get("default_model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

