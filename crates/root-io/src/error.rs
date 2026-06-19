use std::error::Error;
use std::fmt;
use std::num::{ParseFloatError, ParseIntError, TryFromIntError};

pub type Result<T> = std::result::Result<T, RootError>;

#[derive(Debug)]
pub enum RootError {
    Io(std::io::Error),
    Parse(String),
    Format(fmt::Error),
    Decompression(String),
    UnsupportedCompression(String),
    IntConversion(TryFromIntError),
    Regex(regex::Error),
    ParseFloat(ParseFloatError),
    ParseInt(ParseIntError),
    Other(String),
}

impl RootError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }

    pub fn decompression(message: impl Into<String>) -> Self {
        Self::Decompression(message.into())
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Parse(message) => write!(f, "{message}"),
            Self::Format(err) => write!(f, "{err}"),
            Self::Decompression(message) => write!(f, "{message}"),
            Self::UnsupportedCompression(magic) => {
                write!(f, "unsupported ROOT compression algorithm `{magic}`")
            }
            Self::IntConversion(err) => write!(f, "{err}"),
            Self::Regex(err) => write!(f, "{err}"),
            Self::ParseFloat(err) => write!(f, "{err}"),
            Self::ParseInt(err) => write!(f, "{err}"),
            Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl Error for RootError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Format(err) => Some(err),
            Self::IntConversion(err) => Some(err),
            Self::Regex(err) => Some(err),
            Self::ParseFloat(err) => Some(err),
            Self::ParseInt(err) => Some(err),
            Self::Parse(_)
            | Self::Decompression(_)
            | Self::UnsupportedCompression(_)
            | Self::Other(_) => None,
        }
    }
}

impl From<std::io::Error> for RootError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<fmt::Error> for RootError {
    fn from(err: fmt::Error) -> Self {
        Self::Format(err)
    }
}

impl From<TryFromIntError> for RootError {
    fn from(err: TryFromIntError) -> Self {
        Self::IntConversion(err)
    }
}

impl From<regex::Error> for RootError {
    fn from(err: regex::Error) -> Self {
        Self::Regex(err)
    }
}

impl From<ParseFloatError> for RootError {
    fn from(err: ParseFloatError) -> Self {
        Self::ParseFloat(err)
    }
}

impl From<ParseIntError> for RootError {
    fn from(err: ParseIntError) -> Self {
        Self::ParseInt(err)
    }
}
