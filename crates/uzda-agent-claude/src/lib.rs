mod runner;

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use uzda_core::{
    AgentCapabilities, AgentDescriptor, AgentEvent, AgentKind, AgentRuntime, AgentSession,
    RuntimeAvailability,
};

pub use runner::ClaudeError;

#[derive(Debug, Clone)]
pub struct ClaudeAgent {
    command: String,
}

impl ClaudeAgent {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn run_turn(
        &self,
        session: Option<&AgentSession>,
        cwd: &Path,
        prompt: &str,
        on_event: impl FnMut(AgentEvent),
    ) -> Result<AgentSession, ClaudeError> {
        runner::run_turn(&self.command, session, cwd, prompt, on_event)
    }
}

impl Default for ClaudeAgent {
    fn default() -> Self {
        Self::new("claude")
    }
}

impl AgentRuntime for ClaudeAgent {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            kind: AgentKind::Claude,
            display_name: "Claude",
            capabilities: AgentCapabilities::coding_agent(),
        }
    }

    fn availability(&self) -> RuntimeAvailability {
        detect_version(&self.command)
    }
}

fn detect_version(command: &str) -> RuntimeAvailability {
    // This check stays deliberately small; authentication belongs to Claude itself for now.
    match Command::new(command).arg("--version").output() {
        Ok(output) if output.status.success() => RuntimeAvailability::Available {
            version: output_text(&output.stdout),
        },
        Ok(output) => RuntimeAvailability::Unavailable {
            reason: output_text(&output.stderr)
                .unwrap_or_else(|| format!("process exited with {}", output.status)),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => RuntimeAvailability::NotFound,
        Err(error) => RuntimeAvailability::Unavailable {
            reason: error.to_string(),
        },
    }
}

fn output_text(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_claude_runtime() {
        let descriptor = ClaudeAgent::default().descriptor();

        assert_eq!(descriptor.kind, AgentKind::Claude);
        assert_eq!(descriptor.display_name, "Claude");
    }

    #[test]
    fn reports_missing_command() {
        let agent = ClaudeAgent::new("uzda-test-command-that-does-not-exist");

        assert_eq!(agent.availability(), RuntimeAvailability::NotFound);
    }
}
