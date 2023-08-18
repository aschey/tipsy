use std::env::temp_dir;
use std::ffi::CString;
use std::io::{self, Error};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::Stream;
use libc::chmod;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{UnixListener, UnixStream};

use crate::{ConnectionId, ConnectionType, IntoIpcPath, IpcEndpoint, IpcSecurity};

/// Socket permissions and ownership on UNIX
pub struct SecurityAttributes {
    // read/write permissions for owner, group and others in unix octal.
    mode: Option<u16>,
}

impl SecurityAttributes {
    /// called in unix, after server socket has been created
    /// will apply security attributes to the socket.
    fn apply_permissions(&self, path: &str) -> io::Result<()> {
        if let Some(mode) = self.mode {
            let path = CString::new(path)?;
            if unsafe { chmod(path.as_ptr(), mode.into()) } == -1 {
                return Err(Error::last_os_error());
            }
        }

        Ok(())
    }
}

impl IpcSecurity for SecurityAttributes {
    fn empty() -> Self {
        SecurityAttributes { mode: Some(0o600) }
    }

    fn allow_everyone_connect(mut self) -> io::Result<Self> {
        self.mode = Some(0o666);
        Ok(self)
    }

    fn set_mode(mut self, mode: u16) -> io::Result<Self> {
        self.mode = Some(mode);
        Ok(self)
    }

    fn allow_everyone_create() -> io::Result<Self> {
        Ok(SecurityAttributes { mode: None })
    }
}

impl IntoIpcPath for ConnectionId {
    fn into_ipc_path(self) -> PathBuf {
        #[cfg(target_os = "macos")]
        match dirs::home_dir() {
            Some(home) => home.join(format!("Library/Caches/TemporaryItems/{}.sock", self.0)),
            None => temp_dir().join(format!("{}.sock", self.0)),
        }

        #[cfg(not(target_os = "macos"))]
        match dirs::runtime_dir() {
            Some(runtime_dir) => runtime_dir.join(format!("{}.sock", self.0)),
            None => temp_dir().join(format!("{}.sock", self.0)),
        }
    }
}

/// Endpoint implementation for unix systems
pub struct Endpoint {
    path: PathBuf,
    security_attributes: SecurityAttributes,
}

impl Endpoint {
    /// Create a listener from an existing [UnixListener](std::os::unix::net::UnixListener)
    pub fn from_std_listener(
        listener: std::os::unix::net::UnixListener,
    ) -> io::Result<impl Stream<Item = std::io::Result<impl AsyncRead + AsyncWrite>> + 'static>
    {
        let listener = UnixListener::from_std(listener)?;
        Ok(IpcStream {
            path: None,
            listener,
        })
    }

    /// Inner platform-dependant state of the endpoint
    fn inner(&self) -> io::Result<UnixListener> {
        UnixListener::bind(&self.path)
    }

    /// Create a stream from an existing [UnixStream](std::os::unix::net::UnixStream)
    pub async fn from_std_stream(stream: std::os::unix::net::UnixStream) -> io::Result<Connection> {
        Ok(Connection::wrap(UnixStream::from_std(stream)?))
    }
}

#[async_trait]
impl IpcEndpoint for Endpoint {
    /// Stream of incoming connections
    fn incoming(self) -> io::Result<IpcStream> {
        let listener = self.inner()?;
        // the call to bind in `inner()` creates the file
        // `apply_permission()` will set the file permissions.
        self.security_attributes
            .apply_permissions(&self.path.to_string_lossy())?;
        Ok(IpcStream {
            path: Some(self.path),
            listener,
        })
    }

    /// Set security attributes for the connection
    fn set_security_attributes(&mut self, security_attributes: SecurityAttributes) {
        self.security_attributes = security_attributes;
    }

    /// Make new connection using the provided path and running event pool
    async fn connect(
        path: impl IntoIpcPath,
        connection_type: ConnectionType,
    ) -> io::Result<Connection> {
        Ok(Connection::wrap(
            UnixStream::connect(path.into_ipc_path()).await?,
        ))
    }
    /// Returns the path of the endpoint.
    fn path(&self) -> &Path {
        &self.path
    }

    /// New IPC endpoint at the given path
    fn new(endpoint: impl IntoIpcPath, connection_type: ConnectionType) -> Self {
        Endpoint {
            path: endpoint.into_ipc_path(),
            security_attributes: SecurityAttributes::empty(),
        }
    }
}

/// Stream of incoming connections.
///
/// Removes the bound socket file when dropped.
pub struct IpcStream {
    path: Option<PathBuf>,
    listener: UnixListener,
}

impl Stream for IpcStream {
    type Item = io::Result<UnixStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::into_inner(self);
        match Pin::new(&mut this.listener).poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => Poll::Ready(Some(result.map(|(stream, _addr)| stream))),
        }
    }
}

impl Drop for IpcStream {
    fn drop(&mut self) {
        use std::fs;
        if let Some(path) = &self.path {
            if let Ok(()) = fs::remove_file(path) {
                tracing::trace!("Removed socket file at: {:?}", path)
            }
        }
    }
}

/// IPC connection.
pub struct Connection {
    inner: UnixStream,
}

impl Connection {
    fn wrap(stream: UnixStream) -> Self {
        Self { inner: stream }
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.inner).poll_read(ctx, buf)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.inner).poll_write(ctx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.inner).poll_flush(ctx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.inner).poll_shutdown(ctx)
    }
}
