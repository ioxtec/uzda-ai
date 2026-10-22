use crate::{AgentKind, UzdaError};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, UzdaError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(UzdaError::EmptySessionId);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub id: SessionId,
    pub agent: AgentKind,
}

impl AgentSession {
    pub const fn new(id: SessionId, agent: AgentKind) -> Self {
        Self { id, agent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_session_id() {
        let result = SessionId::new("   ");

        assert_eq!(result, Err(UzdaError::EmptySessionId));
    }

    #[test]
    fn keeps_agent_with_its_external_session_id() {
        let id = SessionId::new("thread-123").expect("the ID should be valid");
        let session = AgentSession::new(id, AgentKind::Codex);

        assert_eq!(session.id.as_str(), "thread-123");
        assert_eq!(session.agent, AgentKind::Codex);
    }
}
