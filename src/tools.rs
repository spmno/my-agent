use anyhow::Result;
use rig_core::tool::Tool;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
#[error("tool error: {0}")]
struct ToolError(String);

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Error = ToolError;
    type Args = ReadFileArgs;
    type Output = String;

    fn description(&self) -> String {
        "Read a UTF-8 text file from the project worktree.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path to the file" }
            },
            "required": ["path"],
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        std::fs::read_to_string(&args.path).map_err(|e| ToolError(e.to_string()))
    }
}

#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    old: String,
    new: String,
}

struct EditFile;

impl Tool for EditFile {
    const NAME: &'static str = "edit_file";
    type Error = ToolError;
    type Args = EditFileArgs;
    type Output = String;

    fn description(&self) -> String {
        "Replace the first occurrence of `old` with `new` in a file.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old": { "type": "string", "description": "Exact text to replace" },
                "new": { "type": "string", "description": "Replacement text" }
            },
            "required": ["path", "old", "new"],
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = std::fs::read_to_string(&args.path).map_err(|e| ToolError(e.to_string()))?;
        if !content.contains(&args.old) {
            return Err(ToolError("old text not found in file".into()));
        }
        let updated = content.replacen(&args.old, &args.new, 1);
        std::fs::write(&args.path, updated).map_err(|e| ToolError(e.to_string()))?;
        Ok(format!("edited {}", args.path))
    }
}

#[derive(Deserialize)]
struct BashArgs {
    command: String,
}

struct RunBash;

impl Tool for RunBash {
    const NAME: &'static str = "run_bash";
    type Error = ToolError;
    type Args = BashArgs;
    type Output = String;

    fn description(&self) -> String {
        "Run a shell command in the project worktree and return stdout+stderr.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"],
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .output()
            .map_err(|e| ToolError(e.to_string()))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        Ok(format!(
            "exit={}\nstdout:\n{}\nstderr:\n{}",
            out.status.code().unwrap_or(-1),
            stdout,
            stderr
        ))
    }
}

/// Builtin tools every Builder agent gets. Extension (Phase 4) appends more and
/// persists them to a manifest that loads on boot.
pub fn builtin_tools() -> Result<Vec<Box<dyn rig_core::tool::ToolDyn>>> {
    Ok(vec![
        Box::new(ReadFile),
        Box::new(EditFile),
        Box::new(RunBash),
    ])
}
