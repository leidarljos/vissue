//! Matchable errors so callers do not parse [`Display`] text.

use std::fmt;

/// Recoverable catalog and mutation failures with a stable shape.
#[derive(Debug)]
pub enum Error {
    IssueNotFound { id: String },
    ClaimConflict { id: String, holder: String },
    BlockerCycle { blocker: String, issue: String },
    InvalidState { id: String, state: String },
    Other(anyhow::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::IssueNotFound { id } => write!(f, "issue {id} not found"),
            Error::ClaimConflict { id, holder } => write!(
                f,
                "{id} is claimed by {holder}; pass --force to take it over"
            ),
            Error::BlockerCycle { blocker, issue } if blocker == issue => {
                write!(f, "issue {issue} cannot block itself")
            }
            Error::BlockerCycle { blocker, issue } => {
                write!(
                    f,
                    "adding {blocker} -> {issue} would create a blocker cycle"
                )
            }
            Error::InvalidState { id, state } => {
                write!(f, "{id} is already {state}; cannot claim")
            }
            Error::Other(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Other(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        match err.downcast::<Error>() {
            Ok(typed) => typed,
            Err(other) => Error::Other(other),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Other(err.into())
    }
}
