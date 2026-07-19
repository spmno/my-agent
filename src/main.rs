mod memory;
mod registry;
mod providers;
mod reviewer;
mod tools;
mod evolution;

use anyhow::Result;
use evolution::{
    prompt_evolve::PromptEvolver,
    self_modify,
    tool_ext,
};
use registry::{AgentRegistryConfig, AgentRegistry, Orchestrator};
use reviewer::ReviewGate;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let reg_cfg = AgentRegistryConfig::load("agent.toml")?;
    let threshold = load_escalation_threshold()?;
    let registry = AgentRegistry::new(reg_cfg);
    let orchestrator = Orchestrator::new(registry.clone());
    let reviewer = ReviewGate::new(registry.clone());
    let evolver = PromptEvolver::new(registry.clone(), "AGENTS.md".to_string());

    let mem = memory::MemoryStore::new(&load_memory_cfg()?)?;

    println!("my-agent ready (OpenRouter). Commands: evolve | evolve-code | add-tool | quit");

    let mut line = String::new();
    loop {
        print!("> ");
        io::stdout().flush()?;
        line.clear();
        io::stdin().read_line(&mut line)?;
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "quit" {
            break;
        }

        // REPL meta-commands for the self-evolution features.
        // Order matters: "evolve-code"/"add-tool" must be checked before the
        // "evolve" prefix, since "evolve-code" starts with "evolve".
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

        mem.append_turn(&memory::Turn {
            role: "user".into(),
            content: input.into(),
            ts: now(),
        })?;

        match orchestrator.handle(input).await {
            Ok(out) => {
                let mut out = out;
                if registry::classify(input) == registry::Intent::Implement {
                    match reviewer.review(input, &out).await? {
                        reviewer::Verdict::Approve => {
                            out.push_str("\n\n[review: APPROVED]");
                            mem.record_lesson(&memory::Lesson {
                                summary: format!("implemented + approved: {input}"),
                                ts: now(),
                            })?;
                        }
                        reviewer::Verdict::Reject(fb) => {
                            out.push_str(&format!("\n\n[review: REJECTED] {fb}"));
                            if mem.observe_rule(&fb, threshold)? {
                                mem.promote_rule_to_agents_md(&fb, "AGENTS.md")?;
                                println!("[rule escalated to AGENTS.md]");
                            }
                        }
                        reviewer::Verdict::Clarify(q) => {
                            out.push_str(&format!("\n\n[review: CLARIFY] {q}"));
                        }
                    }
                }
                println!("{out}");
                mem.append_turn(&memory::Turn {
                    role: "agent".into(),
                    content: out,
                    ts: now(),
                })?;
            }
            Err(e) => eprintln!("error: {e}"),
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

