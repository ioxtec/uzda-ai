mod app_server;

use std::io::ErrorKind;
use std::process::Command;

use uzda_core::{AgentCapabilities, AgentDescriptor, AgentKind, AgentRuntime, RuntimeAvailability};

pub use app_server::{CodexAppServer, CodexClientInfo, CodexError, CodexServerInfo};

#[derive(Debug, Clone)]
pub struct CodexAgent {
    command: String,
}

impl CodexAgent {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn connect(&self) -> Result<CodexAppServer, CodexError> {
        CodexAppServer::start(&self.command, CodexClientInfo::default())
    }
}

impl Default for CodexAgent {
    fn default() -> Self {
        Self::new("codex")
    }
}

impl AgentRuntime for CodexAgent {
    fn descriptor(&self) -> AgentDescriptor {
        AgentDescriptor {
            kind: AgentKind::Codex,
            display_name: "Codex",
            capabilities: AgentCapabilities::coding_agent(),
        }
    }

    fn availability(&self) -> RuntimeAvailability {
        detect_version(&self.command)
    }
}

fn detect_version(command: &str) -> RuntimeAvailability {
    // Asking for a version is enough to prove the tool can actually start.
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
    fn describes_codex_runtime() {
        let descriptor = CodexAgent::default().descriptor();

        assert_eq!(descriptor.kind, AgentKind::Codex);
        assert_eq!(descriptor.display_name, "Codex");
    }

    #[test]
    fn reports_missing_command() {
        let agent = CodexAgent::new("uzda-test-command-that-does-not-exist");

        assert_eq!(agent.availability(), RuntimeAvailability::NotFound);
    }
}
