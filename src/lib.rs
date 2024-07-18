//! Tokio IPC transport. Under the hood uses Unix Domain Sockets for Linux/Mac
//! and Named Pipes for Windows.

#![deny(missing_docs)]
#![forbid(clippy::unwrap_used)]
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::doc_markdown)]
#![warn(clippy::default_trait_access)]
#![warn(clippy::ignored_unit_patterns)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::missing_fields_in_debug)]
#![warn(clippy::use_self)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod win;

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

mod platform {
    #[cfg(unix)]
    pub(crate) use crate::unix::{
        from_std_stream, Connection, Endpoint, IpcStream, SecurityAttributes,
    };
    #[cfg(windows)]
    pub(crate) use crate::win::{Connection, Endpoint, IpcStream, SecurityAttributes};
}

/// Path used for an IPC client or server.
pub trait IntoIpcPath: Send {
    /// Converts the object into an IPC path.
    fn into_ipc_path(self) -> io::Result<PathBuf>;
}

impl IntoIpcPath for PathBuf {
    fn into_ipc_path(self) -> io::Result<PathBuf> {
        Ok(self)
    }
}

/// How to proceed when the socket path already exists
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum OnConflict {
    /// Ignore the conflicting socket and continue
    Ignore,
    /// Throw an error when attempting to bind to the path
    Error,
    /// Overwrite the existing socket
    Overwrite,
}

/// Cross-platform representation of an IPC connection path
///
/// Calling [`IntoIpcPath::into_ipc_path`] on this struct will generate a platform-specific IPC
/// path.
///
/// Windows: `\\.\pipe\{serverId}`
///
/// Mac: `$HOME/Library/Caches/TemporaryItems/{serverId}` (defaults to tmp if this directory does
/// not exist)
///
/// Linux: `$XDG_RUNTIME_DIR/{serverId}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerId<T>(pub T)
where
    T: Into<String> + Send;

impl<T> IntoIpcPath for ServerId<T>
where
    T: Into<String> + Send,
{
    fn into_ipc_path(self) -> io::Result<PathBuf> {
        self.into_ipc_path()
    }
}

/// Permissions and ownership for the IPC connection
pub struct SecurityAttributes(platform::SecurityAttributes);

impl SecurityAttributes {
    /// New default security attributes.
    pub fn empty() -> Self {
        Self(platform::SecurityAttributes::empty())
    }

    /// New default security attributes that allow everyone to connect.
    pub fn allow_everyone_connect(self) -> io::Result<Self> {
        Ok(Self(self.0.allow_everyone_connect()?))
    }

    /// Set a custom permission on the socket.
    pub fn set_mode(self, mode: u16) -> io::Result<Self> {
        Ok(Self(self.0.set_mode(mode)?))
    }

    /// New default security attributes that allow everyone to create.
    pub fn allow_everyone_create() -> io::Result<Self> {
        Ok(Self(platform::SecurityAttributes::allow_everyone_create()?))
    }
}

/// IPC endpoint.
pub struct Endpoint(platform::Endpoint);

impl Endpoint {
    /// Stream of incoming connections
    pub fn incoming(self) -> io::Result<IpcStream> {
        Ok(IpcStream(self.0.incoming()?))
    }
    /// Set security attributes for the connection
    pub fn security_attributes(mut self, security_attributes: SecurityAttributes) -> Self {
        self.0 = self.0.security_attributes(security_attributes.0);
        self
    }
    /// Returns the path of the endpoint.
    pub fn path(&self) -> &Path {
        self.0.path()
    }
    /// Make new connection using the provided path and running event pool.
    pub async fn connect(path: impl IntoIpcPath) -> io::Result<Connection> {
        Ok(Connection(platform::Endpoint::connect(path).await?))
    }

    /// New IPC endpoint at the given path
    pub fn new(path: impl IntoIpcPath, on_conflict: OnConflict) -> io::Result<Self> {
        Ok(Self(platform::Endpoint::new(path, on_conflict)?))
    }
}

/// IPC connection.
pub struct Connection(platform::Connection);

impl Connection {
    /// Create a stream from an existing [`UnixStream`](std::os::unix::net::UnixStream).
    #[cfg(unix)]
    pub async fn from_std_stream(stream: std::os::unix::net::UnixStream) -> io::Result<Self> {
        Ok(Self(platform::from_std_stream(stream).await?))
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.0).poll_read(ctx, buf)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.0).poll_write(ctx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.0).poll_flush(ctx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.0).poll_shutdown(ctx)
    }
}

/// Stream of incoming connections.
pub struct IpcStream(platform::IpcStream);

impl IpcStream {
    /// Create a listener from an existing [`UnixListener`](std::os::unix::net::UnixListener).
    #[cfg(unix)]
    pub fn from_std_listener(listener: std::os::unix::net::UnixListener) -> io::Result<Self> {
        Ok(Self(platform::IpcStream::from_std_listener(listener)?))
    }
}

impl Stream for IpcStream {
    type Item = io::Result<Connection>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.0).poll_next(cx).map_ok(Connection)
    }
}
