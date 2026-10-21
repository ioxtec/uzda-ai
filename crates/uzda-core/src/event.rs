// Adapters translate their native messages into this small, predictable stream.
// That way every future UI can focus on presenting events instead of decoding them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    TextDelta(String),
    ToolStarted { name: String },
    ToolCompleted { name: String, succeeded: bool },
    ApprovalRequired { description: String },
    TurnCompleted,
    TurnFailed { message: String },
}
