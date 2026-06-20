use std::error::Error as StdError;
use std::fmt;

pub type Result<T> = std::result::Result<T, WorkflowError>;

#[derive(Debug)]
pub enum WorkflowError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Core(nano_core::NanoError),
    Root(nano_io::RootError),
    InvalidCache(String),
    Assertion(String),
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Core(error) => write!(f, "{error}"),
            Self::Root(error) => write!(f, "{error}"),
            Self::InvalidCache(message) => write!(f, "{message}"),
            Self::Assertion(message) => write!(f, "{message}"),
        }
    }
}

impl StdError for WorkflowError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Core(error) => Some(error),
            Self::Root(error) => Some(error),
            Self::InvalidCache(_) | Self::Assertion(_) => None,
        }
    }
}

impl From<std::io::Error> for WorkflowError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for WorkflowError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<nano_core::NanoError> for WorkflowError {
    fn from(value: nano_core::NanoError) -> Self {
        Self::Core(value)
    }
}

impl From<nano_io::RootError> for WorkflowError {
    fn from(value: nano_io::RootError) -> Self {
        Self::Root(value)
    }
}
