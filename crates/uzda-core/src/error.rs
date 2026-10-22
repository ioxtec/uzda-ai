use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UzdaError {
    EmptySessionId,
}

impl fmt::Display for UzdaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySessionId => formatter.write_str("session ID cannot be empty"),
        }
    }
}

impl Error for UzdaError {}
