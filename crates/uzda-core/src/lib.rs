mod agent;
mod error;
mod event;
mod session;

pub use agent::{AgentCapabilities, AgentDescriptor, AgentKind, AgentRuntime, RuntimeAvailability};
pub use error::UzdaError;
pub use event::AgentEvent;
pub use session::{AgentSession, SessionId};

pub fn welcome_message() -> &'static str {
    "Welcome to Uzda!"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_welcome_message() {
        assert_eq!(welcome_message(), "Welcome to Uzda!");
    }
}
