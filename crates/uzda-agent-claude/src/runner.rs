use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use uzda_core::{AgentEvent, AgentKind, AgentSession, SessionId};

pub(crate) fn run_turn(
    command: &str,
    session: Option<&AgentSession>,
    cwd: &Path,
    prompt: &str,
    mut on_event: impl FnMut(AgentEvent),
) -> Result<AgentSession, ClaudeError> {
    validate_input(session, cwd, prompt)?;

    let mut process = Command::new(command);
    process
        .current_dir(cwd)
        .args([
            "-p",
            prompt,
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompts",
            "none",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if let Some(session) = session {
        process.args(["--resume", session.id.as_str()]);
    }

    let mut child = process.spawn().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ClaudeError::NotFound
        } else {
            ClaudeError::Io(error)
        }
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ClaudeError::Protocol("stdout pipe is missing".to_owned()))?;
    let mut stream = ClaudeStream::default();

    for line in BufReader::new(stdout).lines() {
        let line = line?;

        if line.trim().is_empty() {
            continue;
        }

        let message: Value = serde_json::from_str(&line)?;
        for event in stream.handle_message(&message)? {
            on_event(event);
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(ClaudeError::ProcessFailed(status.code()));
    }

    if !stream.completed {
        return Err(ClaudeError::MissingResult);
    }

    if let Some(message) = stream.failure {
        return Err(ClaudeError::TurnFailed(message));
    }

    let session_id = stream.session_id.ok_or(ClaudeError::MissingSessionId)?;
    let id =
        SessionId::new(session_id).map_err(|error| ClaudeError::Protocol(error.to_string()))?;

    Ok(AgentSession::new(id, AgentKind::Claude))
}

fn validate_input(
    session: Option<&AgentSession>,
    cwd: &Path,
    prompt: &str,
) -> Result<(), ClaudeError> {
    if let Some(session) = session
        && session.agent != AgentKind::Claude
    {
        return Err(ClaudeError::WrongAgent(session.agent));
    }

    if !cwd.is_absolute() {
        return Err(ClaudeError::InvalidWorkingDirectory(cwd.to_owned()));
    }

    if prompt.trim().is_empty() {
        return Err(ClaudeError::EmptyPrompt);
    }

    Ok(())
}

#[derive(Default)]
struct ClaudeStream {
    session_id: Option<String>,
    active_tools: HashMap<String, String>,
    completed: bool,
    failure: Option<String>,
}

impl ClaudeStream {
    fn handle_message(&mut self, message: &Value) -> Result<Vec<AgentEvent>, ClaudeError> {
        self.capture_session_id(message);

        match message.get("type").and_then(Value::as_str) {
            Some("stream_event") => self.handle_stream_event(&message["event"]),
            Some("user") => Ok(self.handle_tool_results(&message["message"]["content"])),
            Some("result") => self.handle_result(message),
            _ => Ok(Vec::new()),
        }
    }

    fn capture_session_id(&mut self, message: &Value) {
        if self.session_id.is_none() {
            self.session_id = message
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
    }

    fn handle_stream_event(&mut self, event: &Value) -> Result<Vec<AgentEvent>, ClaudeError> {
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                let text = event["delta"]["text"]
                    .as_str()
                    .ok_or_else(|| ClaudeError::Protocol("text delta has no text".to_owned()))?;

                Ok(vec![AgentEvent::TextDelta(text.to_owned())])
            }
            Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
                let id = event["content_block"]["id"]
                    .as_str()
                    .ok_or_else(|| ClaudeError::Protocol("tool call has no ID".to_owned()))?;
                let name = event["content_block"]["name"]
                    .as_str()
                    .ok_or_else(|| ClaudeError::Protocol("tool call has no name".to_owned()))?;

                self.active_tools.insert(id.to_owned(), name.to_owned());
                Ok(vec![AgentEvent::ToolStarted {
                    name: name.to_owned(),
                }])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn handle_tool_results(&mut self, content: &Value) -> Vec<AgentEvent> {
        let Some(blocks) = content.as_array() else {
            return Vec::new();
        };

        blocks
            .iter()
            .filter_map(|block| {
                if block["type"] != "tool_result" {
                    return None;
                }

                let tool_id = block["tool_use_id"].as_str()?;
                let name = self.active_tools.remove(tool_id)?;
                let succeeded = !block["is_error"].as_bool().unwrap_or(false);

                Some(AgentEvent::ToolCompleted { name, succeeded })
            })
            .collect()
    }

    fn handle_result(&mut self, message: &Value) -> Result<Vec<AgentEvent>, ClaudeError> {
        if self.completed {
            return Ok(Vec::new());
        }

        self.completed = true;
        let is_error = message["is_error"].as_bool().unwrap_or(false);
        let subtype = message["subtype"].as_str().unwrap_or("unknown");

        if !is_error && subtype == "success" {
            return Ok(vec![AgentEvent::TurnCompleted]);
        }

        let message = message["result"]
            .as_str()
            .filter(|message| !message.trim().is_empty())
            .unwrap_or(subtype)
            .to_owned();
        self.failure = Some(message.clone());

        Ok(vec![AgentEvent::TurnFailed { message }])
    }
}

#[derive(Debug)]
pub enum ClaudeError {
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    ProcessFailed(Option<i32>),
    TurnFailed(String),
    MissingResult,
    MissingSessionId,
    NotFound,
    EmptyPrompt,
    WrongAgent(AgentKind),
    InvalidWorkingDirectory(PathBuf),
}

impl fmt::Display for ClaudeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "Claude I/O error: {error}"),
            Self::Json(error) => write!(formatter, "invalid message from Claude: {error}"),
            Self::Protocol(message) => write!(formatter, "unexpected Claude response: {message}"),
            Self::ProcessFailed(Some(code)) => {
                write!(formatter, "Claude process exited with code {code}")
            }
            Self::ProcessFailed(None) => formatter.write_str("Claude process ended unexpectedly"),
            Self::TurnFailed(message) => write!(formatter, "Claude turn failed: {message}"),
            Self::MissingResult => formatter.write_str("Claude returned no final result"),
            Self::MissingSessionId => formatter.write_str("Claude returned no session ID"),
            Self::NotFound => formatter.write_str("Claude CLI is not installed or not in PATH"),
            Self::EmptyPrompt => formatter.write_str("prompt cannot be empty"),
            Self::WrongAgent(agent) => {
                write!(formatter, "cannot use a {agent} session with Claude")
            }
            Self::InvalidWorkingDirectory(path) => {
                write!(
                    formatter,
                    "working directory must be absolute: {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ClaudeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Protocol(_)
            | Self::ProcessFailed(_)
            | Self::TurnFailed(_)
            | Self::MissingResult
            | Self::MissingSessionId
            | Self::NotFound
            | Self::EmptyPrompt
            | Self::WrongAgent(_)
            | Self::InvalidWorkingDirectory(_) => None,
        }
    }
}

impl From<io::Error> for ClaudeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ClaudeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_text_and_captures_the_session() {
        let mut stream = ClaudeStream::default();
        let message = serde_json::json!({
            "type": "stream_event",
            "session_id": "session-123",
            "event": {
                "type": "content_block_delta",
                "delta": {
                    "type": "text_delta",
                    "text": "Hello"
                }
            }
        });

        let events = stream
            .handle_message(&message)
            .expect("the event should parse");

        assert_eq!(stream.session_id.as_deref(), Some("session-123"));
        assert_eq!(events, vec![AgentEvent::TextDelta("Hello".to_owned())]);
    }

    #[test]
    fn maps_a_tool_lifecycle() {
        let mut stream = ClaudeStream::default();
        let started = serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": {
                    "type": "tool_use",
                    "id": "tool-1",
                    "name": "Bash"
                }
            }
        });
        let completed = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool-1",
                    "is_error": false
                }]
            }
        });

        assert_eq!(
            stream.handle_message(&started).unwrap(),
            vec![AgentEvent::ToolStarted {
                name: "Bash".to_owned()
            }]
        );
        assert_eq!(
            stream.handle_message(&completed).unwrap(),
            vec![AgentEvent::ToolCompleted {
                name: "Bash".to_owned(),
                succeeded: true
            }]
        );
    }

    #[test]
    fn recognizes_a_successful_result() {
        let mut stream = ClaudeStream::default();
        let message = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "session_id": "session-123"
        });

        assert_eq!(
            stream.handle_message(&message).unwrap(),
            vec![AgentEvent::TurnCompleted]
        );
    }

    #[test]
    fn keeps_the_failure_from_a_result() {
        let mut stream = ClaudeStream::default();
        let message = serde_json::json!({
            "type": "result",
            "subtype": "error_max_turns",
            "is_error": true,
            "result": "Turn limit reached",
            "session_id": "session-123"
        });

        assert_eq!(
            stream.handle_message(&message).unwrap(),
            vec![AgentEvent::TurnFailed {
                message: "Turn limit reached".to_owned()
            }]
        );
        assert_eq!(stream.failure.as_deref(), Some("Turn limit reached"));
    }

    #[test]
    fn rejects_a_codex_session() {
        let session = AgentSession::new(
            SessionId::new("thread-123").expect("the ID should be valid"),
            AgentKind::Codex,
        );

        assert!(matches!(
            validate_input(Some(&session), Path::new("/tmp"), "hello"),
            Err(ClaudeError::WrongAgent(AgentKind::Codex))
        ));
    }
}
