use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const INITIALIZE_REQUEST_ID: u64 = 1;

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

#[derive(Debug)]
pub enum CodexError {
    Io(io::Error),
    Json(serde_json::Error),
    Server { code: i64, message: String },
    Protocol(String),
    ProcessEnded,
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
        }
    }
}

impl Error for CodexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Server { .. } | Self::Protocol(_) | Self::ProcessEnded => None,
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
        };

        server.initialize(client_info)?;
        Ok(server)
    }

    pub fn server_info(&self) -> &CodexServerInfo {
        &self.server_info
    }

    fn initialize(&mut self, client_info: CodexClientInfo) -> Result<(), CodexError> {
        let request = json!({
            "id": INITIALIZE_REQUEST_ID,
            "method": "initialize",
            "params": {
                "clientInfo": client_info,
                "capabilities": {}
            }
        });

        write_message(&mut self.stdin, &request)?;
        self.server_info = read_response(&mut self.stdout, INITIALIZE_REQUEST_ID)?;

        // The notification tells Codex that Uzda is ready to receive normal server events.
        write_message(&mut self.stdin, &json!({ "method": "initialized" }))?;
        Ok(())
    }
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        // There is no conversation to preserve yet, so a direct shutdown keeps this first client simple.
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
) -> Result<T, CodexError> {
    let mut line = String::new();

    loop {
        line.clear();

        if reader.read_line(&mut line)? == 0 {
            return Err(CodexError::ProcessEnded);
        }

        let message: Value = serde_json::from_str(&line)?;

        // Notifications may arrive while a request is in flight. They belong to the event loop,
        // but for now we simply keep reading until our matching response appears.
        if message.get("id").and_then(Value::as_u64) != Some(request_id) {
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

        let response: CodexServerInfo =
            read_response(&mut input, 1).expect("the matching response should be parsed");

        assert_eq!(response.user_agent, "Codex test");
        assert_eq!(response.platform_os, "macos");
    }

    #[test]
    fn returns_server_error() {
        let mut input = Cursor::new("{\"id\":1,\"error\":{\"code\":-1,\"message\":\"nope\"}}\n");

        let result = read_response::<CodexServerInfo>(&mut input, 1);

        assert!(matches!(
            result,
            Err(CodexError::Server {
                code: -1,
                message
            }) if message == "nope"
        ));
    }
}
