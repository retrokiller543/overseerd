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

    #[error("unable to allocate storage for a {len}-byte frame")]
    FrameAllocation { len: usize },

    #[error("frame read made no progress for {idle_timeout:?}")]
    ReadTimeout { idle_timeout: std::time::Duration },

    #[error("connection exceeded its limit of {max} in-flight calls")]
    TooManyCalls { max: usize },

    #[error("peer reused active call id {id}")]
    DuplicateCallId { id: crate::frame::CallId },

    #[error("timed out after {timeout:?} writing a control frame; connection is poisoned")]
    ControlWriteTimeout { timeout: std::time::Duration },

    #[error("connection exceeded its limit of {max} control response tasks")]
    ControlTasksSaturated { max: usize },

    #[error("transport control task failed: {0}")]
    ControlTask(String),

    #[error("unexpected message type")]
    UnexpectedMessage,
}

pub type Result<T> = std::result::Result<T, Error>;
