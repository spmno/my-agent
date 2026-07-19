// Core memory/experience store. Some methods are not yet called by the
// autonomous loop but are part of the self-improvement design (rule escalation,
// lesson recording for the review-gated mode).
#![allow(dead_code)]

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Turn {
    pub role: String, // "user" | "agent"
    pub content: String,
    pub ts: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Lesson {
    pub summary: String,
    pub ts: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Rule {
    pub text: String,
    pub count: u32,
    pub written_to: bool,
}

#[derive(Debug, Deserialize)]
pub struct MemoryConfig {
    pub dir: PathBuf,
    pub conversation_file: String,
    pub lessons_file: String,
    pub rules_file: String,
}

pub struct MemoryStore {
    #[allow(dead_code)]
    dir: PathBuf,
    conversation_file: PathBuf,
    lessons_file: PathBuf,
    rules_file: PathBuf,
}

impl MemoryStore {
    pub fn new(cfg: &MemoryConfig) -> Result<Self> {
        std::fs::create_dir_all(&cfg.dir)?;
        Ok(Self {
            dir: cfg.dir.clone(),
            conversation_file: cfg.dir.join(&cfg.conversation_file),
            lessons_file: cfg.dir.join(&cfg.lessons_file),
            rules_file: cfg.dir.join(&cfg.rules_file),
        })
    }

    pub fn append_turn(&self, turn: &Turn) -> Result<()> {
        let line = serde_json::to_string(turn)?;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.conversation_file)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    pub fn record_lesson(&self, lesson: &Lesson) -> Result<()> {
        let line = serde_json::to_string(lesson)?;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.lessons_file)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// Record an observed behavioral rule. Returns true if it just crossed the
    /// escalation threshold and should be promoted into AGENTS.md.
    pub fn observe_rule(&self, text: &str, threshold: u32) -> Result<bool> {
        let mut rules = self.load_rules()?;
        let existing = rules.iter().position(|r| r.text == text);
        match existing {
            Some(idx) => {
                rules[idx].count += 1;
                let crossed = rules[idx].count >= threshold && !rules[idx].written_to;
                if crossed {
                    rules[idx].written_to = true;
                }
                self.save_rules(&rules)?;
                Ok(crossed)
            }
            None => {
                rules.push(Rule {
                    text: text.to_string(),
                    count: 1,
                    written_to: false,
                });
                let crossed = 1 >= threshold;
                if crossed {
                    rules.last_mut().unwrap().written_to = true;
                }
                self.save_rules(&rules)?;
                Ok(crossed)
            }
        }
    }

    pub fn load_rules(&self) -> Result<Vec<Rule>> {
        if !self.rules_file.exists() {
            return Ok(vec![]);
        }
        let raw = std::fs::read_to_string(&self.rules_file)?;
        let rules: Vec<Rule> = serde_json::from_str(&raw)?;
        Ok(rules)
    }

    /// Append an escalated rule to AGENTS.md so it becomes a persistent
    /// behavioral instruction. Returns the new section text written.
    pub fn promote_rule_to_agents_md(&self, rule: &str, path: &str) -> Result<String> {
        let section = format!("\n## Escalated rule\n- {rule}\n");
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{section}")?;
        Ok(section)
    }

    fn save_rules(&self, rules: &[Rule]) -> Result<()> {
        let raw = serde_json::to_string_pretty(rules)?;
        std::fs::write(&self.rules_file, raw)?;
        Ok(())
    }
}

