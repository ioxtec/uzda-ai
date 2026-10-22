use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{AgentKind, AgentSession, SessionId};

const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSessionStore {
    path: PathBuf,
}

impl LocalSessionStore {
    pub fn for_project(project_root: impl AsRef<Path>) -> Self {
        Self {
            path: project_root.as_ref().join(".uzda").join("sessions.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self, agent: AgentKind) -> Result<Option<AgentSession>, SessionStoreError> {
        let sessions = self.read_file()?;
        let Some(stored) = sessions
            .sessions
            .into_iter()
            .find(|session| session.agent == agent.as_str())
        else {
            return Ok(None);
        };

        let id = SessionId::new(stored.id)
            .map_err(|error| SessionStoreError::InvalidSession(error.to_string()))?;

        Ok(Some(AgentSession::new(id, agent)))
    }

    pub fn save(&self, session: &AgentSession) -> Result<(), SessionStoreError> {
        let mut sessions = self.read_file()?;
        let stored = StoredSession {
            agent: session.agent.as_str().to_owned(),
            id: session.id.as_str().to_owned(),
        };

        if let Some(existing) = sessions
            .sessions
            .iter_mut()
            .find(|entry| entry.agent == stored.agent)
        {
            *existing = stored;
        } else {
            sessions.sessions.push(stored);
        }

        if let Some(directory) = self.path.parent() {
            fs::create_dir_all(directory)?;
        }

        // A trailing newline keeps the file pleasant to inspect by hand.
        let mut contents = serde_json::to_string_pretty(&sessions)?;
        contents.push('\n');
        fs::write(&self.path, contents)?;

        Ok(())
    }

    fn read_file(&self) -> Result<StoredSessions, SessionStoreError> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredSessions::default());
            }
            Err(error) => return Err(error.into()),
        };
        let sessions: StoredSessions = serde_json::from_str(&contents)?;

        if sessions.version != STORE_VERSION {
            return Err(SessionStoreError::UnsupportedVersion(sessions.version));
        }

        Ok(sessions)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSessions {
    version: u32,
    sessions: Vec<StoredSession>,
}

impl Default for StoredSessions {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            sessions: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSession {
    agent: String,
    id: String,
}

#[derive(Debug)]
pub enum SessionStoreError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidSession(String),
    UnsupportedVersion(u32),
}

impl fmt::Display for SessionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "session storage I/O error: {error}"),
            Self::Json(error) => write!(formatter, "invalid session storage file: {error}"),
            Self::InvalidSession(message) => write!(formatter, "invalid stored session: {message}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported session storage version: {version}")
            }
        }
    }
}

impl Error for SessionStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::InvalidSession(_) | Self::UnsupportedVersion(_) => None,
        }
    }
}

impl From<io::Error> for SessionStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for SessionStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn saves_and_loads_sessions_by_agent() {
        let directory = test_directory();
        let store = LocalSessionStore::for_project(&directory);
        let codex = AgentSession::new(
            SessionId::new("codex-thread").expect("the ID should be valid"),
            AgentKind::Codex,
        );
        let claude = AgentSession::new(
            SessionId::new("claude-session").expect("the ID should be valid"),
            AgentKind::Claude,
        );

        store.save(&codex).expect("the session should be saved");
        store.save(&claude).expect("the session should be saved");

        assert_eq!(store.load(AgentKind::Codex).unwrap(), Some(codex));
        assert_eq!(store.load(AgentKind::Claude).unwrap(), Some(claude));

        fs::remove_dir_all(directory).expect("the test directory should be removable");
    }

    #[test]
    fn missing_file_means_there_is_no_session_yet() {
        let directory = test_directory();
        let store = LocalSessionStore::for_project(directory);

        assert_eq!(store.load(AgentKind::Codex).unwrap(), None);
    }

    #[test]
    fn replaces_the_previous_session_for_an_agent() {
        let directory = test_directory();
        let store = LocalSessionStore::for_project(&directory);
        let first = AgentSession::new(
            SessionId::new("first-thread").expect("the ID should be valid"),
            AgentKind::Codex,
        );
        let second = AgentSession::new(
            SessionId::new("second-thread").expect("the ID should be valid"),
            AgentKind::Codex,
        );

        store.save(&first).expect("the session should be saved");
        store.save(&second).expect("the session should be replaced");

        assert_eq!(store.load(AgentKind::Codex).unwrap(), Some(second));

        fs::remove_dir_all(directory).expect("the test directory should be removable");
    }

    fn test_directory() -> PathBuf {
        let number = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "uzda-session-store-{}-{number}",
            std::process::id()
        ))
    }
}
