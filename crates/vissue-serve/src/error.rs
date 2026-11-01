//! Owner-process failures.

use std::fmt;

/// Bind, spawn, and catalog failures for the owner.
#[derive(Debug)]
pub enum Error {
    /// A tracker read or mutation failed.
    Core(vissue_core::error::Error),
    /// Socket, file, process, or notify failure.
    Other(anyhow::Error),
}

/// Library result using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Core(err) => write!(f, "{err}"),
            Error::Other(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Core(err) => Some(err),
            Error::Other(err) => Some(err.as_ref()),
        }
    }
}

impl From<vissue_core::error::Error> for Error {
    fn from(err: vissue_core::error::Error) -> Self {
        Error::Core(err)
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        match err.downcast::<vissue_core::error::Error>() {
            Ok(core) => Error::Core(core),
            Err(other) => Error::Other(other),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Other(err.into())
    }
}

#[cfg(unix)]
impl From<notify::Error> for Error {
    fn from(err: notify::Error) -> Self {
        Error::Other(err.into())
    }
}

impl From<fmt::Error> for Error {
    fn from(err: fmt::Error) -> Self {
        Error::Other(err.into())
    }
}
