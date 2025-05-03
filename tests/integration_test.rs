use std::io;
use std::time::Duration;

use futures_channel::oneshot;
use futures_util::{Future, StreamExt};
use tipsy::{
    Connection, Endpoint, IntoIpcPath, IpcStream, OnConflict, SecurityAttributes, ServerId,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, split};

fn dummy_endpoint(base: &str) -> ServerId<String> {
    let num: u64 = rand::Rng::random(&mut rand::rng());
    ServerId::new(format!("{base}-{num}"))
}

async fn run_server(endpoint: Endpoint) {
    let endpoint =
        endpoint.security_attributes(SecurityAttributes::empty().set_mode(0o777).unwrap());
    let incoming = endpoint.incoming().expect("failed to open up a new socket");

    run_stream(incoming).await;
}

async fn run_stream(incoming: IpcStream) {
    futures_util::pin_mut!(incoming);
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

async fn run_clients<F, Fut>(create_client: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Connection, io::Error>>,
{
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("Connecting to client 0...");
    let mut client_0 = create_client().await.expect("failed to open client_0");
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("Connecting to client 1...");
    let mut client_1 = create_client().await.expect("failed to open client_1");
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
}

#[tokio::test]
async fn single_id() {
    let endpoint = Endpoint::new(dummy_endpoint("test"), OnConflict::Overwrite).unwrap();
    smoke_test(endpoint).await;
}

#[tokio::test]
async fn nested_path() {
    let endpoint = Endpoint::new(dummy_endpoint("test/test1"), OnConflict::Overwrite).unwrap();
    smoke_test(endpoint).await;
}

// Windows named paths don't exist in the filesystem so this test is only valid on Unix
#[cfg(unix)]
#[tokio::test]
async fn error_on_path_exists() {
    let path = dummy_endpoint("test");
    let mut incoming = Endpoint::new(path.clone(), OnConflict::Error)
        .unwrap()
        .incoming()
        .unwrap();
    tokio::spawn(async move {
        incoming.next().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(Endpoint::new(path, OnConflict::Error).is_err());
}

#[tokio::test]
async fn ok_on_path_overwrite() {
    let path = dummy_endpoint("test");
    let mut incoming = Endpoint::new(path.clone(), OnConflict::Overwrite)
        .unwrap()
        .incoming()
        .unwrap();
    tokio::spawn(async move {
        incoming.next().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(Endpoint::new(path, OnConflict::Overwrite).is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn std_listener_stream() {
    let path = dummy_endpoint("test").into_ipc_path().unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let incoming = IpcStream::from_std_listener(listener).unwrap();
    tokio::spawn(async move {
        tokio::select! {
            () = run_stream(incoming) => {},
            _ = shutdown_rx => {}
        }
    });

    run_clients(|| {
        Connection::from_std_stream(std::os::unix::net::UnixStream::connect(path.clone()).unwrap())
    })
    .await;
    let _ = shutdown_tx.send(());
}

async fn smoke_test(endpoint: Endpoint) {
    let path = endpoint.path().to_path_buf();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(async move {
        tokio::select! {
            () = run_server(endpoint) => {}
            _ = shutdown_rx => {}
        }
    });

    run_clients(|| Endpoint::connect(path.clone())).await;

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

    let path = dummy_endpoint("test");
    let endpoint = Endpoint::new(path, OnConflict::Overwrite).unwrap();
    is_static(endpoint.incoming());
}

fn create_endpoint_with_permissions(attr: SecurityAttributes) -> ::std::io::Result<()> {
    let path = dummy_endpoint("test");

    let endpoint = Endpoint::new(path, OnConflict::Overwrite)
        .unwrap()
        .security_attributes(attr);
    endpoint.incoming().map(|_| ())
}

#[tokio::test]
async fn test_endpoint_permissions() {
    create_endpoint_with_permissions(SecurityAttributes::empty())
        .expect("failed with no attributes");
    create_endpoint_with_permissions(SecurityAttributes::allow_everyone_create().unwrap())
        .expect("failed with attributes for creating");
    create_endpoint_with_permissions(
        SecurityAttributes::empty()
            .allow_everyone_connect()
            .unwrap(),
    )
    .expect("failed with attributes for connecting");
}

#[cfg(unix)]
#[test]
fn set_server_id_directory() {
    let path = ServerId::new("test")
        .parent_folder("/tmp")
        .into_ipc_path()
        .unwrap();
    assert_eq!("/tmp/test.sock", path.to_string_lossy());
}
