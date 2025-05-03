use std::env::temp_dir;
use std::ffi::CString;
use std::fs;
use std::io::{self, Error};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use libc::chmod;
use tokio::net::{UnixListener, UnixStream};
use tracing::trace;

use crate::{IntoIpcPath, OnConflict, ServerId};

pub(crate) struct SecurityAttributes {
    // read/write permissions for owner, group and others in unix octal.
    mode: Option<u16>,
}

impl SecurityAttributes {
    fn apply_permissions(&self, path: &str) -> io::Result<()> {
        if let Some(mode) = self.mode {
            let path = CString::new(path)?;
            // mode_t doesn't need into() on mac but does on linux
            #[allow(clippy::useless_conversion)]
            if unsafe { chmod(path.as_ptr(), mode.into()) } == -1 {
                return Err(Error::last_os_error());
            }
        }

        Ok(())
    }

    pub(crate) fn empty() -> Self {
        Self { mode: Some(0o600) }
    }

    pub(crate) fn allow_everyone_connect(mut self) -> io::Result<Self> {
        self.mode = Some(0o666);
        Ok(self)
    }

    pub(crate) fn set_mode(mut self, mode: u16) -> io::Result<Self> {
        self.mode = Some(mode);
        Ok(self)
    }

    pub(crate) fn allow_everyone_create() -> io::Result<Self> {
        Ok(Self { mode: None })
    }
}

impl<T> ServerId<T>
where
    T: Into<String> + Send,
{
    pub(crate) fn into_ipc_path(self) -> io::Result<PathBuf> {
        let sock_name = format!("{}.sock", self.id.into());
        let path = self
            .parent_folder
            .or_else(dirs::runtime_dir)
            .unwrap_or_else(temp_dir)
            .join(sock_name);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(path)
    }
}

/// Endpoint implementation for unix systems
pub(crate) struct Endpoint {
    path: PathBuf,
    security_attributes: SecurityAttributes,
}

impl Endpoint {
    /// Inner platform-dependant state of the endpoint
    pub(crate) fn inner(&self) -> io::Result<UnixListener> {
        UnixListener::bind(&self.path)
    }

    pub(crate) fn incoming(self) -> io::Result<IpcStream> {
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

    pub(crate) fn security_attributes(mut self, security_attributes: SecurityAttributes) -> Self {
        self.security_attributes = security_attributes;
        self
    }

    pub(crate) async fn connect(path: impl IntoIpcPath) -> io::Result<Connection> {
        UnixStream::connect(path.into_ipc_path()?).await
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn new(endpoint: impl IntoIpcPath, on_conflict: OnConflict) -> io::Result<Self> {
        let path = endpoint.into_ipc_path()?;
        if std::path::Path::new(&path).exists() {
            match on_conflict {
                OnConflict::Error => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("Unable to bind to {path:?} because the path already exists"),
                    ));
                }
                OnConflict::Overwrite => {
                    fs::remove_file(&path)?;
                }
                OnConflict::Ignore => {}
            }
        }

        Ok(Self {
            path,
            security_attributes: SecurityAttributes::empty(),
        })
    }
}

pub(crate) async fn from_std_stream(
    stream: std::os::unix::net::UnixStream,
) -> io::Result<Connection> {
    stream.set_nonblocking(true)?;
    UnixStream::from_std(stream)
}

pub(crate) struct IpcStream {
    path: Option<PathBuf>,
    listener: UnixListener,
}

impl IpcStream {
    pub(crate) fn from_std_listener(
        listener: std::os::unix::net::UnixListener,
    ) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        let listener = UnixListener::from_std(listener)?;
        Ok(Self {
            path: None,
            listener,
        })
    }
}

pub(crate) type Connection = UnixStream;

impl Stream for IpcStream {
    type Item = io::Result<Connection>;

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
        if let Some(path) = &self.path {
            if let Ok(()) = fs::remove_file(path) {
                trace!("Removed socket file at: {:?}", path);
            }
        }
    }
}
