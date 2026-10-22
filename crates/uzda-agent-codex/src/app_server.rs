use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uzda_core::{AgentEvent, AgentKind, AgentSession, ApprovalDecision, SessionId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexClientInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

impl Default for CodexClientInfo {
    fn default() -> Self {
        Self {
            name: "uzda".to_owned(),
            title: Some("Uzda".to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexServerInfo {
    pub user_agent: String,
    pub codex_home: String,
    pub platform_family: String,
    pub platform_os: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexThreadOptions {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub ephemeral: bool,
}

impl CodexThreadOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            model: None,
            ephemeral: false,
        }
    }

    fn validate(&self) -> Result<(), CodexError> {
        if self.cwd.is_absolute() {
            Ok(())
        } else {
            Err(CodexError::InvalidWorkingDirectory(self.cwd.clone()))
        }
    }
}

#[derive(Debug)]
pub enum CodexError {
    Io(io::Error),
    Json(serde_json::Error),
    Server { code: i64, message: String },
    Protocol(String),
    ProcessEnded,
    EmptyPrompt,
    WrongAgent(AgentKind),
    TurnFailed(String),
    UnsupportedServerRequest(String),
    InvalidWorkingDirectory(PathBuf),
}

impl fmt::Display for CodexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "Codex I/O error: {error}"),
            Self::Json(error) => write!(formatter, "invalid message from Codex: {error}"),
            Self::Server { code, message } => {
                write!(formatter, "Codex server error {code}: {message}")
            }
            Self::Protocol(message) => write!(formatter, "unexpected Codex response: {message}"),
            Self::ProcessEnded => formatter.write_str("Codex app-server ended unexpectedly"),
            Self::EmptyPrompt => formatter.write_str("prompt cannot be empty"),
            Self::WrongAgent(agent) => {
                write!(formatter, "cannot use a {agent} session with Codex")
            }
            Self::TurnFailed(message) => write!(formatter, "Codex turn failed: {message}"),
            Self::UnsupportedServerRequest(method) => {
                write!(formatter, "Codex requested unsupported action: {method}")
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

impl Error for CodexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Server { .. }
            | Self::Protocol(_)
            | Self::ProcessEnded
            | Self::EmptyPrompt
            | Self::WrongAgent(_)
            | Self::TurnFailed(_)
            | Self::UnsupportedServerRequest(_)
            | Self::InvalidWorkingDirectory(_) => None,
        }
    }
}

impl From<io::Error> for CodexError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CodexError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct CodexAppServer {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    server_info: CodexServerInfo,
    next_request_id: u64,
    pending_messages: VecDeque<Value>,
}

impl CodexAppServer {
    pub fn start(command: &str, client_info: CodexClientInfo) -> Result<Self, CodexError> {
        let mut child = Command::new(command)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CodexError::Protocol("stdin pipe is missing".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CodexError::Protocol("stdout pipe is missing".to_owned()))?;

        let mut server = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            server_info: CodexServerInfo {
                user_agent: String::new(),
                codex_home: String::new(),
                platform_family: String::new(),
                platform_os: String::new(),
            },
            next_request_id: 1,
            pending_messages: VecDeque::new(),
        };

        server.initialize(client_info)?;
        Ok(server)
    }

    pub fn server_info(&self) -> &CodexServerInfo {
        &self.server_info
    }

    pub fn start_thread(
        &mut self,
        options: CodexThreadOptions,
    ) -> Result<AgentSession, CodexError> {
        options.validate()?;

        let response: ThreadResponse = self.request(
            "thread/start",
            json!({
                "cwd": options.cwd,
                "model": options.model,
                "ephemeral": options.ephemeral
            }),
        )?;

        let session_id = SessionId::new(response.thread.id)
            .map_err(|error| CodexError::Protocol(error.to_string()))?;

        Ok(AgentSession::new(session_id, AgentKind::Codex))
    }

    pub fn resume_thread(&mut self, session_id: &SessionId) -> Result<AgentSession, CodexError> {
        let response: ThreadResponse = self.request(
            "thread/resume",
            json!({
                "threadId": session_id.as_str(),
                "excludeTurns": true
            }),
        )?;
        let resumed_id = SessionId::new(response.thread.id)
            .map_err(|error| CodexError::Protocol(error.to_string()))?;

        Ok(AgentSession::new(resumed_id, AgentKind::Codex))
    }

    pub fn run_turn(
        &mut self,
        session: &AgentSession,
        prompt: &str,
        mut on_event: impl FnMut(AgentEvent),
        mut on_approval: impl FnMut(&str) -> ApprovalDecision,
    ) -> Result<(), CodexError> {
        if session.agent != AgentKind::Codex {
            return Err(CodexError::WrongAgent(session.agent));
        }

        if prompt.trim().is_empty() {
            return Err(CodexError::EmptyPrompt);
        }

        let response: TurnStartResponse = self.request(
            "turn/start",
            json!({
                "threadId": session.id.as_str(),
                "input": [{
                    "type": "text",
                    "text": prompt
                }]
            }),
        )?;
        let turn_id = response.turn.id;
        let mut latest_error = None;

        loop {
            let message = self.next_message()?;

            if message.get("id").is_some() && message.get("method").is_some() {
                self.handle_server_request(
                    message,
                    session.id.as_str(),
                    &turn_id,
                    &mut on_event,
                    &mut on_approval,
                )?;
                continue;
            }

            match message.get("method").and_then(Value::as_str) {
                Some("item/agentMessage/delta") => {
                    let event: AgentMessageDelta = parse_params(message)?;

                    if event.matches(session.id.as_str(), &turn_id) {
                        on_event(AgentEvent::TextDelta(event.delta));
                    }
                }
                Some("item/started") => {
                    if let Some(event) = tool_event(message, session.id.as_str(), &turn_id, true)? {
                        on_event(event);
                    }
                }
                Some("item/completed") => {
                    if let Some(event) = tool_event(message, session.id.as_str(), &turn_id, false)?
                    {
                        on_event(event);
                    }
                }
                Some("error") => {
                    let event: ErrorNotification = parse_params(message)?;

                    if event.matches(session.id.as_str(), &turn_id) {
                        latest_error = Some(event.error.message);
                    }
                }
                Some("turn/completed") => {
                    let event: TurnCompleted = parse_params(message)?;

                    if !event.matches(session.id.as_str(), &turn_id) {
                        continue;
                    }

                    if event.turn.status == "completed" {
                        on_event(AgentEvent::TurnCompleted);
                        return Ok(());
                    }

                    let message = event
                        .turn
                        .error
                        .map(|error| error.message)
                        .or(latest_error)
                        .unwrap_or_else(|| format!("turn ended with status {}", event.turn.status));
                    on_event(AgentEvent::TurnFailed {
                        message: message.clone(),
                    });
                    return Err(CodexError::TurnFailed(message));
                }
                _ => {}
            }
        }
    }

    fn handle_server_request(
        &mut self,
        message: Value,
        thread_id: &str,
        turn_id: &str,
        on_event: &mut impl FnMut(AgentEvent),
        on_approval: &mut impl FnMut(&str) -> ApprovalDecision,
    ) -> Result<(), CodexError> {
        let method = message["method"].as_str().unwrap_or("unknown").to_owned();

        if !matches!(
            method.as_str(),
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
        ) {
            return Err(CodexError::UnsupportedServerRequest(method));
        }

        let request_id = message["id"].clone();
        let params: ApprovalRequest = parse_params(message)?;

        if !params.matches(thread_id, turn_id) {
            write_approval_response(&mut self.stdin, request_id, ApprovalDecision::Decline)?;
            return Ok(());
        }

        let description = params.description(&method);
        on_event(AgentEvent::ApprovalRequired {
            description: description.clone(),
        });
        let decision = on_approval(&description);
        write_approval_response(&mut self.stdin, request_id, decision)
    }

    fn initialize(&mut self, client_info: CodexClientInfo) -> Result<(), CodexError> {
        self.server_info = self.request(
            "initialize",
            json!({
                "clientInfo": client_info,
                "capabilities": {}
            }),
        )?;

        // The notification tells Codex that Uzda is ready to receive normal server events.
        write_message(&mut self.stdin, &json!({ "method": "initialized" }))?;
        Ok(())
    }

    fn request<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, CodexError> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let request = json!({
            "id": request_id,
            "method": method,
            "params": params
        });

        write_message(&mut self.stdin, &request)?;
        read_response(&mut self.stdout, request_id, &mut self.pending_messages)
    }

    fn next_message(&mut self) -> Result<Value, CodexError> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(message);
        }

        read_message(&mut self.stdout)
    }
}

#[derive(Debug, Deserialize)]
struct ThreadResponse {
    thread: StartedThread,
}

#[derive(Debug, Deserialize)]
struct StartedThread {
    id: String,
}

#[derive(Debug, Deserialize)]
struct TurnStartResponse {
    turn: Turn,
}

#[derive(Debug, Deserialize)]
struct Turn {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<TurnError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentMessageDelta {
    delta: String,
    thread_id: String,
    turn_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemNotification {
    item: Value,
    thread_id: String,
    turn_id: String,
}

impl ItemNotification {
    fn matches(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id == thread_id && self.turn_id == turn_id
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalRequest {
    thread_id: String,
    turn_id: String,
    command: Option<String>,
    reason: Option<String>,
    grant_root: Option<String>,
}

impl ApprovalRequest {
    fn matches(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id == thread_id && self.turn_id == turn_id
    }

    fn description(&self, method: &str) -> String {
        let action = match method {
            "item/commandExecution/requestApproval" => self
                .command
                .as_deref()
                .map(|command| format!("Run command: {command}"))
                .unwrap_or_else(|| "Run a command".to_owned()),
            "item/fileChange/requestApproval" => self
                .grant_root
                .as_deref()
                .map(|root| format!("Allow file changes in: {root}"))
                .unwrap_or_else(|| "Apply file changes".to_owned()),
            _ => "Perform an action".to_owned(),
        };

        match self.reason.as_deref() {
            Some(reason) if !reason.trim().is_empty() => format!("{action}\nReason: {reason}"),
            _ => action,
        }
    }
}

impl AgentMessageDelta {
    fn matches(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id == thread_id && self.turn_id == turn_id
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnCompleted {
    thread_id: String,
    turn: Turn,
}

impl TurnCompleted {
    fn matches(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id == thread_id && self.turn.id == turn_id
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorNotification {
    error: TurnError,
    thread_id: String,
    turn_id: String,
}

impl ErrorNotification {
    fn matches(&self, thread_id: &str, turn_id: &str) -> bool {
        self.thread_id == thread_id && self.turn_id == turn_id
    }
}

#[derive(Debug, Deserialize)]
struct TurnError {
    message: String,
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        // Threads live in Codex, so stopping this transport does not discard their history.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), CodexError> {
    serde_json::to_writer(&mut *writer, message)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read_response<T: DeserializeOwned>(
    reader: &mut impl BufRead,
    request_id: u64,
    pending_messages: &mut VecDeque<Value>,
) -> Result<T, CodexError> {
    loop {
        let message = read_message(reader)?;

        // Codex can start streaming before it acknowledges the request. Keep those early events.
        if message.get("id").and_then(Value::as_u64) != Some(request_id) {
            pending_messages.push_back(message);
            continue;
        }

        if let Some(error) = message.get("error") {
            return Err(server_error(error));
        }

        let result = message
            .get("result")
            .cloned()
            .ok_or_else(|| CodexError::Protocol("response has no result".to_owned()))?;

        return serde_json::from_value(result).map_err(CodexError::from);
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Value, CodexError> {
    let mut line = String::new();

    if reader.read_line(&mut line)? == 0 {
        return Err(CodexError::ProcessEnded);
    }

    serde_json::from_str(&line).map_err(CodexError::from)
}

fn parse_params<T: DeserializeOwned>(message: Value) -> Result<T, CodexError> {
    let params = message
        .get("params")
        .cloned()
        .ok_or_else(|| CodexError::Protocol("notification has no params".to_owned()))?;

    serde_json::from_value(params).map_err(CodexError::from)
}

fn tool_event(
    message: Value,
    thread_id: &str,
    turn_id: &str,
    started: bool,
) -> Result<Option<AgentEvent>, CodexError> {
    let notification: ItemNotification = parse_params(message)?;

    if !notification.matches(thread_id, turn_id) {
        return Ok(None);
    }

    let Some(name) = tool_name(&notification.item) else {
        return Ok(None);
    };

    if started {
        return Ok(Some(AgentEvent::ToolStarted { name }));
    }

    let succeeded =
        notification.item["status"] == "completed" || notification.item["success"] == true;

    Ok(Some(AgentEvent::ToolCompleted { name, succeeded }))
}

fn tool_name(item: &Value) -> Option<String> {
    match item.get("type")?.as_str()? {
        "commandExecution" => item["command"].as_str().map(ToOwned::to_owned),
        "fileChange" => Some("file changes".to_owned()),
        "mcpToolCall" => Some(format!(
            "{}/{}",
            item["server"].as_str()?,
            item["tool"].as_str()?
        )),
        "dynamicToolCall" => item["tool"].as_str().map(ToOwned::to_owned),
        "collabAgentToolCall" => item["tool"].as_str().map(ToOwned::to_owned),
        "webSearch" => Some("web search".to_owned()),
        "imageView" => Some("image view".to_owned()),
        "imageGeneration" => Some("image generation".to_owned()),
        _ => None,
    }
}

fn write_approval_response(
    writer: &mut impl Write,
    request_id: Value,
    decision: ApprovalDecision,
) -> Result<(), CodexError> {
    let decision = match decision {
        ApprovalDecision::Approve => "accept",
        ApprovalDecision::Decline => "decline",
    };

    write_message(
        writer,
        &json!({
            "id": request_id,
            "result": { "decision": decision }
        }),
    )
}

fn server_error(error: &Value) -> CodexError {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_owned();

    CodexError::Server { code, message }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn writes_one_json_message_per_line() {
        let mut output = Vec::new();

        write_message(&mut output, &json!({ "method": "initialized" }))
            .expect("the message should be written");

        assert_eq!(output, b"{\"method\":\"initialized\"}\n");
    }

    #[test]
    fn waits_for_matching_response() {
        let messages = concat!(
            "{\"method\":\"account/updated\",\"params\":{}}\n",
            "{\"id\":1,\"result\":{",
            "\"userAgent\":\"Codex test\",",
            "\"codexHome\":\"/tmp/codex\",",
            "\"platformFamily\":\"unix\",",
            "\"platformOs\":\"macos\"}}\n"
        );
        let mut input = Cursor::new(messages);
        let mut pending_messages = VecDeque::new();

        let response: CodexServerInfo = read_response(&mut input, 1, &mut pending_messages)
            .expect("the matching response should be parsed");

        assert_eq!(response.user_agent, "Codex test");
        assert_eq!(response.platform_os, "macos");
        assert_eq!(pending_messages.len(), 1);
        assert_eq!(pending_messages[0]["method"], "account/updated");
    }

    #[test]
    fn returns_server_error() {
        let mut input = Cursor::new("{\"id\":1,\"error\":{\"code\":-1,\"message\":\"nope\"}}\n");
        let mut pending_messages = VecDeque::new();

        let result = read_response::<CodexServerInfo>(&mut input, 1, &mut pending_messages);

        assert!(matches!(
            result,
            Err(CodexError::Server {
                code: -1,
                message
            }) if message == "nope"
        ));
    }

    #[test]
    fn maps_codex_thread_to_core_session() {
        let response: ThreadResponse = serde_json::from_value(json!({
            "thread": { "id": "thread-123" }
        }))
        .expect("the response should be valid");
        let session_id = SessionId::new(response.thread.id).expect("the ID should be valid");
        let session = AgentSession::new(session_id, AgentKind::Codex);

        assert_eq!(session.id.as_str(), "thread-123");
        assert_eq!(session.agent, AgentKind::Codex);
    }

    #[test]
    fn maps_resumed_thread_to_core_session() {
        let response: ThreadResponse = serde_json::from_value(json!({
            "thread": {
                "id": "thread-123",
                "turns": []
            },
            "model": "test-model"
        }))
        .expect("the response should be valid");
        let session_id = SessionId::new(response.thread.id).expect("the ID should be valid");
        let session = AgentSession::new(session_id, AgentKind::Codex);

        assert_eq!(session.id.as_str(), "thread-123");
    }

    #[test]
    fn rejects_relative_working_directory() {
        let options = CodexThreadOptions::new("relative/path");

        assert!(matches!(
            options.validate(),
            Err(CodexError::InvalidWorkingDirectory(path))
                if path == std::path::Path::new("relative/path")
        ));
    }

    #[test]
    fn parses_agent_message_delta() {
        let message = json!({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": "Hello",
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        });

        let event: AgentMessageDelta = parse_params(message).expect("the event should be valid");

        assert_eq!(event.delta, "Hello");
        assert!(event.matches("thread-1", "turn-1"));
    }

    #[test]
    fn recognizes_successful_turn_completion() {
        let message = json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "items": [],
                    "status": "completed"
                }
            }
        });

        let event: TurnCompleted = parse_params(message).expect("the event should be valid");

        assert_eq!(event.turn.status, "completed");
        assert!(event.matches("thread-1", "turn-1"));
    }

    #[test]
    fn maps_command_lifecycle_to_agent_events() {
        let started = json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": {
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "cargo check",
                    "commandActions": [],
                    "cwd": "/tmp",
                    "status": "inProgress"
                }
            }
        });
        let completed = json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "item": {
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "cargo check",
                    "commandActions": [],
                    "cwd": "/tmp",
                    "status": "completed"
                }
            }
        });

        assert_eq!(
            tool_event(started, "thread-1", "turn-1", true).unwrap(),
            Some(AgentEvent::ToolStarted {
                name: "cargo check".to_owned()
            })
        );
        assert_eq!(
            tool_event(completed, "thread-1", "turn-1", false).unwrap(),
            Some(AgentEvent::ToolCompleted {
                name: "cargo check".to_owned(),
                succeeded: true
            })
        );
    }

    #[test]
    fn writes_a_narrow_approval_decision() {
        let mut output = Vec::new();

        write_approval_response(&mut output, json!(7), ApprovalDecision::Approve)
            .expect("the response should be written");

        assert_eq!(output, b"{\"id\":7,\"result\":{\"decision\":\"accept\"}}\n");
    }
}
