use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),

    Parse {
        offset: usize,
        message: String,
    },

    Decompression(String),

    UnsupportedCompression(String),

    MissingTree(String),

    MissingBranch(String),

    TypeMismatch {
        branch: String,
        root_type: String,
        requested: &'static str,
    },

    UnsupportedLayout {
        what: String,
        reason: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Parse { offset, message } => {
                write!(f, "parse error at byte {offset}: {message}")
            }
            Self::Decompression(message) => write!(f, "decompression error: {message}"),
            Self::UnsupportedCompression(algo) => {
                write!(f, "unsupported compression algorithm {algo}")
            }
            Self::MissingTree(name) => write!(f, "tree {name:?} not found"),
            Self::MissingBranch(name) => write!(f, "branch {name:?} not found"),
            Self::TypeMismatch {
                branch,
                root_type,
                requested,
            } => write!(
                f,
                "branch {branch:?} has ROOT type {root_type:?}, not requested scalar type {requested}"
            ),
            Self::UnsupportedLayout { what, reason } => {
                write!(f, "unsupported layout for {what}: {reason}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl Error {
    pub(crate) fn parse(offset: usize, message: impl Into<String>) -> Self {
        Self::Parse {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(what: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::UnsupportedLayout {
            what: what.into(),
            reason: reason.into(),
        }
    }
}
