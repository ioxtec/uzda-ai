use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Codex,
    Claude,
}

impl AgentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub streams_events: bool,
    pub resumes_sessions: bool,
    pub requests_approval: bool,
}

impl AgentCapabilities {
    pub const fn coding_agent() -> Self {
        Self {
            streams_events: true,
            resumes_sessions: true,
            requests_approval: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDescriptor {
    pub kind: AgentKind,
    pub display_name: &'static str,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAvailability {
    Available { version: Option<String> },
    NotFound,
    Unavailable { reason: String },
}

impl RuntimeAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

/// A small common surface that keeps the UI independent from a specific agent.
pub trait AgentRuntime {
    fn descriptor(&self) -> AgentDescriptor;
    fn availability(&self) -> RuntimeAvailability;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_kind_has_stable_text_identifier() {
        assert_eq!(AgentKind::Codex.as_str(), "codex");
        assert_eq!(AgentKind::Claude.as_str(), "claude");
    }

    #[test]
    fn available_runtime_reports_its_state() {
        let availability = RuntimeAvailability::Available { version: None };

        assert!(availability.is_available());
    }
}
