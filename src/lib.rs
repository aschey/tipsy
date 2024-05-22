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

use futures::Future;
#[cfg(unix)]
pub use unix::{Connection, Endpoint, IpcStream, SecurityAttributes};
#[cfg(windows)]
pub use win::{Connection, Endpoint, IpcStream, SecurityAttributes};

/// Endpoint trait shared by Windows and Unix implementations
pub trait IpcEndpoint: Send + Sized {
    /// Stream of incoming connections
    fn incoming(self) -> io::Result<IpcStream>;
    /// Set security attributes for the connection
    fn set_security_attributes(&mut self, security_attributes: SecurityAttributes);
    /// Returns the path of the endpoint.
    fn path(&self) -> &Path;
    /// Make new connection using the provided path and running event pool.
    fn connect(path: impl IntoIpcPath) -> impl Future<Output = io::Result<Connection>> + Send;
    // async fn connect_messages(path: impl IntoIpcPath) -> io::Result<Connection>;
    /// New IPC endpoint at the given path
    fn new(path: impl IntoIpcPath, on_conflict: OnConflict) -> io::Result<Self>;
}

/// Security trait used by Windows and Unix implementations
pub trait IpcSecurity: Send + Sized {
    /// New default security attributes.
    fn empty() -> Self;
    /// New default security attributes that allow everyone to connect.
    fn allow_everyone_connect(self) -> io::Result<Self>;
    /// Set a custom permission on the socket
    fn set_mode(self, mode: u16) -> io::Result<Self>;
    /// New default security attributes that allow everyone to create.
    fn allow_everyone_create() -> io::Result<Self>;
}

/// Trait representing a path used for an IPC client or server
pub trait IntoIpcPath: Send {
    /// Convert the object into an IPC path
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

/// Cross-platform representation of an IPC connection
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerId<T>(pub T)
where
    T: Into<String> + Send;
