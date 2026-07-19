use anyhow::Result;
use rig_core::{client::{ProviderClient, CompletionClient}, providers::openrouter};

/// 构建 OpenRouter 客户端。rig 自带原生的 `providers::openrouter` 客户端
/// （OpenRouter 提供 OpenAI 兼容接口，并通过模型 slug 路由到 deepseek / glm /
/// kimi 等供应商）。API Key 从环境变量 OPENROUTER_API_KEY 读取。
pub fn openrouter_client() -> Result<openrouter::Client> {
    let client = openrouter::Client::from_env()?;
    Ok(client)
}

/// 对话型 Agent 的别名：基于 OpenRouter CompletionModel 的 rig Agent。
pub type ChatAgent = rig_core::agent::Agent<openrouter::CompletionModel>;

// 预留给 Phase-3 的公开辅助函数（在 registry 之外构造按角色划分的 Agent）。
#[allow(dead_code)]
pub fn build_agent(client: &openrouter::Client, model: &str, preamble: &str) -> ChatAgent {
    client
        .agent(model)
        .preamble(preamble)
        .temperature(0.7)
        .build()
}

