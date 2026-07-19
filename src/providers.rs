use anyhow::Result;
use rig_core::{client::{ProviderClient, CompletionClient}, providers::openrouter};

/// Build an OpenRouter client. rig ships a native `providers::openrouter` client
/// (OpenRouter exposes an OpenAI-compatible API and routes to deepseek / glm /
/// kimi / etc. via model slugs). Reads OPENROUTER_API_KEY from the environment.
pub fn openrouter_client() -> Result<openrouter::Client> {
    let client = openrouter::Client::from_env()?;
    Ok(client)
}

pub type ChatAgent = rig_core::agent::Agent<openrouter::CompletionModel>;

// Public helper kept for Phase-3 (per-role agent construction outside the registry).
#[allow(dead_code)]
pub fn build_agent(client: &openrouter::Client, model: &str, preamble: &str) -> ChatAgent {
    client
        .agent(model)
        .preamble(preamble)
        .temperature(0.7)
        .build()
}
