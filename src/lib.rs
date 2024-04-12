//! Tokio IPC transport. Under the hood uses Unix Domain Sockets for Linux/Mac
//! and Named Pipes for Windows.

#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

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

/// Endpoint trait shared by windows and unix implementations

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

/// Security trait used by windows and unix implementations
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::channel::oneshot;
    use futures::future::{ready, select, Either};
    use futures::{FutureExt as _, StreamExt as _};
    use tokio::io::{split, AsyncReadExt, AsyncWriteExt};

    use super::{Endpoint, SecurityAttributes};
    use crate::{IntoIpcPath, IpcEndpoint, IpcSecurity, OnConflict, ServerId};

    fn dummy_endpoint() -> ServerId<String> {
        let num: u64 = rand::Rng::gen(&mut rand::thread_rng());
        ServerId(format!("test-{num}"))
    }

    async fn run_server(id: ServerId<String>) {
        let mut endpoint = Endpoint::new(id, OnConflict::Overwrite).unwrap();

        endpoint.set_security_attributes(SecurityAttributes::empty().set_mode(0o777).unwrap());
        let incoming = endpoint.incoming().expect("failed to open up a new socket");
        futures::pin_mut!(incoming);

        while let Some(result) = incoming.next().await {
            match result {
                Ok(stream) => {
                    let (mut reader, mut writer) = split(stream);
                    let mut buf = [0u8; 5];
                    reader
                        .read_exact(&mut buf)
                        .await
                        .expect("unable to read from socket");
                    writer
                        .write_all(&buf[..])
                        .await
                        .expect("unable to write to socket");
                }
                _ => unreachable!("ideally"),
            }
        }
    }

    #[tokio::test]
    async fn smoke_test() {
        let path = dummy_endpoint();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let server = select(Box::pin(run_server(path.clone())), shutdown_rx).then(|either| {
            match either {
                Either::Right((_, server)) => {
                    drop(server);
                }
                _ => unreachable!("also ideally"),
            };
            ready(())
        });
        tokio::spawn(server);

        tokio::time::sleep(Duration::from_secs(2)).await;

        println!("Connecting to client 0...");
        let mut client_0 = Endpoint::connect(path.clone())
            .await
            .expect("failed to open client_0");
        tokio::time::sleep(Duration::from_secs(2)).await;
        println!("Connecting to client 1...");
        let mut client_1 = Endpoint::connect(path.clone())
            .await
            .expect("failed to open client_1");
        let msg = b"hello";

        let mut rx_buf = vec![0u8; msg.len()];
        client_0
            .write_all(msg)
            .await
            .expect("Unable to write message to client");
        client_0
            .read_exact(&mut rx_buf)
            .await
            .expect("Unable to read message from client");

        let mut rx_buf2 = vec![0u8; msg.len()];
        client_1
            .write_all(msg)
            .await
            .expect("Unable to write message to client");
        client_1
            .read_exact(&mut rx_buf2)
            .await
            .expect("Unable to read message from client");

        assert_eq!(rx_buf, msg);
        assert_eq!(rx_buf2, msg);

        // shutdown server
        if let Ok(()) = shutdown_tx.send(()) {
            // wait one second for the file to be deleted.
            tokio::time::sleep(Duration::from_secs(1)).await;
            // assert that it was
            assert!(!path.into_ipc_path().unwrap().exists());
        }
    }

    #[tokio::test]
    async fn incoming_stream_is_static() {
        fn is_static<T: 'static>(_: T) {}

        let path = dummy_endpoint();
        let endpoint = Endpoint::new(path, OnConflict::Overwrite).unwrap();
        is_static(endpoint.incoming());
    }

    #[cfg(windows)]
    fn create_pipe_with_permissions(attr: SecurityAttributes) -> ::std::io::Result<()> {
        let path = dummy_endpoint();

        let mut endpoint = Endpoint::new(path, OnConflict::Overwrite).unwrap();
        endpoint.set_security_attributes(attr);
        endpoint.incoming().map(|_| ())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_pipe_permissions() {
        create_pipe_with_permissions(SecurityAttributes::empty())
            .expect("failed with no attributes");
        create_pipe_with_permissions(SecurityAttributes::allow_everyone_create().unwrap())
            .expect("failed with attributes for creating");
        create_pipe_with_permissions(
            SecurityAttributes::empty()
                .allow_everyone_connect()
                .unwrap(),
        )
        .expect("failed with attributes for connecting");
    }
}
