// DeepSeek 供应商客户端：rig 自带原生 `providers::deepseek` 客户端，
// API Key 从环境变量 DEEPSEEK_API_KEY 读取。DeepSeek API 兼容 OpenAI 格式。
use anyhow::Result;
use rig_core::{client::{ProviderClient, CompletionClient}, providers::deepseek};

/// 构建 DeepSeek 客户端。rig 自带原生的 `providers::deepseek` 客户端
/// （直连 DeepSeek API，不经过 OpenRouter）。API Key 从环境变量
/// DEEPSEEK_API_KEY 读取。
pub fn deepseek_client() -> Result<deepseek::Client> {
    let client = deepseek::Client::from_env()?;
    Ok(client)
}

/// 对话型 Agent 的别名：基于 DeepSeek CompletionModel 的 rig Agent。
pub type ChatAgent = rig_core::agent::Agent<deepseek::CompletionModel>;

// 预留给 Phase-3 的公开辅助函数（在 registry 之外构造按角色划分的 Agent）。
#[allow(dead_code)]
pub fn build_agent(client: &deepseek::Client, model: &str, preamble: &str) -> ChatAgent {
    client
        .agent(model)
        .preamble(preamble)
        .temperature(0.7)
        .build()
}
