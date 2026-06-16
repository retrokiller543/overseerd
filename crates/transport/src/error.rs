use thiserror::Error;

/// Errors produced by transport implementations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("transport is closed")]
    Closed,

    #[error("transport io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("frame too large: {len} bytes exceeds maximum of {max}")]
    FrameTooLarge { len: usize, max: usize },

    #[error("unexpected message type")]
    UnexpectedMessage,
}

pub type Result<T> = std::result::Result<T, Error>;
