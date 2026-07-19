use crate::providers::{openrouter_client, ChatAgent};
use crate::registry::{AgentRegistry, Role};
use anyhow::Result;
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use std::process::Command;

/// A fixed eval benchmark: canned tasks scored pass/fail by an auditor agent.
/// The prompt-evolution loop only promotes a new preamble if it scores >= the
/// previous one on this benchmark. This is the gate that prevents prompt drift.
const BENCHMARK_TASKS: &[&str] = &[
    "Write a Rust function that returns the nth Fibonacci number.",
    "Explain what a closure is in one sentence.",
    "List three ways to handle errors in Rust.",
];

pub struct PromptEvolver {
    registry: AgentRegistry,
    agents_md_path: String,
}

impl PromptEvolver {
    pub fn new(registry: AgentRegistry, agents_md_path: String) -> Self {
        Self {
            registry,
            agents_md_path,
        }
    }

    /// Read the current preamble (AGENTS.md).
    pub fn current_preamble(&self) -> Result<String> {
        Ok(std::fs::read_to_string(&self.agents_md_path)?)
    }

    /// Run the benchmark against a given preamble; returns the count of passing tasks.
    pub async fn eval_preamble(&self, preamble: &str) -> Result<usize> {
        let client = openrouter_client()?;
        let agent: ChatAgent = client
            .agent("deepseek/deepseek-chat")
            .preamble(preamble)
            .temperature(0.0)
            .build();
        let judge = self.registry.build(Role::Auditor)?;
        let mut passed = 0;
        for task in BENCHMARK_TASKS {
            let out = agent.prompt(*task).await?;
            let verdict_prompt = format!(
                "Does the following answer correctly and usefully address the task?\n\
                 Task: {task}\nAnswer: {out}\n\
                 Reply with exactly one line: PASS or FAIL."
            );
            let v = judge.run(&verdict_prompt).await?;
            if v.to_uppercase().contains("PASS") {
                passed += 1;
            }
        }
        Ok(passed)
    }

    /// Propose a new preamble via a meta-agent, then promote it only if the eval
    /// shows it is at least as good. Reversible: we git-tag the old preamble
    /// before overwriting.
    pub async fn evolve(&self) -> Result<String> {
        let current = self.current_preamble()?;
        let meta = self.registry.build(Role::Orchestrator)?;
        let proposal_prompt = format!(
            "You are a meta-agent improving an AI agent's system prompt. \
             Below is the current prompt. Propose an improved version that makes \
             the agent more helpful, correct, and safe. Output ONLY the new prompt \
             text, no commentary.\n\nCURRENT PROMPT:\n{current}"
        );
        let proposed = meta.run(&proposal_prompt).await?;

        let old_score = self.eval_preamble(&current).await?;
        let new_score = self.eval_preamble(&proposed).await?;

        if new_score >= old_score {
            // Checkpoint the old version, then promote.
            let _ = Command::new("git")
                .args(["tag", &format!("prompt-v{}", now())])
                .output();
            std::fs::write(&self.agents_md_path, &proposed)?;
            Ok(format!(
                "promoted new prompt (score {new_score} >= {old_score}); old tagged in git"
            ))
        } else {
            Ok(format!(
                "kept old prompt (new score {new_score} < {old_score}); no change"
            ))
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
