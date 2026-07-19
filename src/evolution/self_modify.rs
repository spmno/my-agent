// 代码自修改模块：对源码做精确 old->new 替换，再用 `cargo build` 验证。
// 失败则回退到修改前内容，并把编译错误返回，便于上层 LLM 自我纠正后重试。
// 这是 OMO 子代理改代码的真实 Rust 版本：编译即验证关卡。
use anyhow::Result;
use std::process::Command;

/// 对源文件做精确 old->new 替换，然后用 `cargo build` 验证。失败时文件回退到
/// 修改前内容，并返回编译错误，以便 LLM 调用方自我纠正后重试。这是 OMO 子代理
/// 编辑代码库的现实 Rust 类比：编译即验证关卡。
pub fn evolve_code(file: &str, old: &str, new: &str) -> Result<String> {
    let content = std::fs::read_to_string(file)?;
    if !content.contains(old) {
        return Err(anyhow::anyhow!("old text not found in {file}"));
    }
    // Back up the pre-edit content so we can restore it deterministically on
    // failure (works regardless of git tracking state).
    let backup = format!("{file}.evo.bak");
    std::fs::write(&backup, &content)?;
    let updated = content.replacen(old, new, 1);
    std::fs::write(file, updated)?;

    match run_build() {
        Ok(out) if out.status.success() => {
            let _ = std::fs::remove_file(&backup);
            let _ = run_tests();
            Ok(format!("evolved {file} (build passed)"))
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            revert(file, &backup);
            Err(anyhow::anyhow!("build failed; reverted {file}. error:\n{stderr}"))
        }
        Err(e) => {
            revert(file, &backup);
            Err(e)
        }
    }
}

fn run_build() -> Result<std::process::Output> {
    Command::new("cargo")
        .args(["build"])
        .output()
        .map_err(|e| anyhow::anyhow!("cargo build failed to spawn: {e}"))
}

fn run_tests() -> Result<std::process::Output> {
    Command::new("cargo")
        .args(["test"])
        .output()
        .map_err(|e| anyhow::anyhow!("cargo test failed to spawn: {e}"))
}

fn revert(file: &str, backup: &str) {
    // Restore the pre-edit content from the backup, then remove the backup.
    if let Ok(prev) = std::fs::read_to_string(backup) {
        let _ = std::fs::write(file, prev);
    }
    let _ = std::fs::remove_file(backup);
}

