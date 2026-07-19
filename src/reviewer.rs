use crate::registry::{AgentRegistry, Role};

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    Reject(String), // feedback to return to the builder
    Clarify(String),
}

/// SDD two-stage review gate. Mirrors OMO's requirement that no task is "done"
/// until an auditor passes both stages:
///   1. Spec compliance  — did it implement what was asked?
///   2. Code quality     — security, correctness, maintainability.
/// Both must Approve, otherwise the work is sent back with feedback.
pub struct ReviewGate {
    registry: AgentRegistry,
}

impl ReviewGate {
    pub fn new(registry: AgentRegistry) -> Self {
        Self { registry }
    }

    pub async fn review(&self, task: &str, produced: &str) -> anyhow::Result<Verdict> {
        let auditor = self.registry.build(Role::Auditor)?;

        let spec_prompt = format!(
            "You are a SPEC COMPLIANCE reviewer (stage 1 of 2). \
             Task requested:\n{task}\n\nWork produced:\n{produced}\n\n\
             Did the work implement what was asked? Reply with exactly one line: \
             'APPROVE' or 'REJECT: <what is missing or wrong>'."
        );
        let spec_out = auditor.run(&spec_prompt).await?;

        if !spec_out.to_uppercase().contains("APPROVE") {
            let fb = spec_out
                .lines()
                .find(|l| l.to_uppercase().contains("REJECT"))
                .unwrap_or("spec compliance failed")
                .to_string();
            return Ok(Verdict::Reject(fb));
        }

        let qual_prompt = format!(
            "You are a CODE QUALITY reviewer (stage 2 of 2). \
             Task requested:\n{task}\n\nWork produced:\n{produced}\n\n\
             Check security, correctness, and maintainability. Reply with exactly one line: \
             'APPROVE' or 'REJECT: <issues>' or 'CLARIFY: <question>'."
        );
        let qual_out = auditor.run(&qual_prompt).await?;
        let up = qual_out.to_uppercase();
        if up.contains("APPROVE") {
            Ok(Verdict::Approve)
        } else if up.contains("CLARIFY") {
            Ok(Verdict::Clarify(
                qual_out.lines().find(|l| l.to_uppercase().contains("CLARIFY")).unwrap_or("").to_string(),
            ))
        } else {
            Ok(Verdict::Reject(
                qual_out.lines().find(|l| l.to_uppercase().contains("REJECT")).unwrap_or("quality failed").to_string(),
            ))
        }
    }
}
