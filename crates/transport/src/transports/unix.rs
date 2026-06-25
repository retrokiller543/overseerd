#![cfg(unix)]

use std::fs::{File, create_dir_all};
use std::path::{Path, PathBuf};

use tokio::net::{
    UnixListener,
    unix::{OwnedReadHalf, OwnedWriteHalf},
};
use tracing::{debug, instrument, trace};

use crate::{
    error::Result,
    frame::PeerInfo,
    transport::Transport,
    transports::stream::{StreamConnection, StreamResponder},
};

/// Unix socket transport. Removes the socket file on drop.
pub struct UnixTransport {
    listener: UnixListener,
    path: PathBuf,
}

/// An accepted Unix socket connection.
pub type UnixConnection = StreamConnection<OwnedReadHalf, OwnedWriteHalf>;

/// Responds to one inbound call on a Unix socket connection.
pub type UnixResponder = StreamResponder<OwnedWriteHalf>;

impl UnixTransport {
    pub fn bind(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        Self::ensure_path(&path)?;

        let listener = UnixListener::bind(&path)?;

        debug!(path = %path.display(), "Unix transport bound");

        Ok(Self { listener, path })
    }

    fn ensure_path(path: &Path) -> Result<()> {
        if !path.exists() {
            Self::create_socket_file(path)?
        }

        Ok(())
    }

    fn create_socket_file(path: &Path) -> Result<()> {
        let parent = path.parent();

        if let Some(parent) = parent
            && !parent.exists()
        {
            create_dir_all(parent)?;
        }

        File::create(path)?;

        Ok(())
    }
}

impl Drop for UnixTransport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        debug!(path = %self.path.display(), "Unix socket removed");
    }
}

impl Transport for UnixTransport {
    type Connection = UnixConnection;

    #[instrument(level = "debug", skip_all)]
    async fn accept(&mut self) -> Result<UnixConnection> {
        trace!("waiting for Unix connection");

        let (stream, _) = self.listener.accept().await?;

        debug!("Unix connection accepted");

        let (read, write) = stream.into_split();
        let peer = PeerInfo { addr: None };

        Ok(StreamConnection::new(read, write, peer))
    }
}
