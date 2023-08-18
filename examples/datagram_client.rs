use parity_tokio_ipc::{ConnectionId, ConnectionType, Endpoint, IpcEndpoint};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Run it with server path to connect as argument");

    let client = Endpoint::connect(ConnectionId(path), ConnectionType::Datagram)
        .await
        .expect("Failed to connect client.");
    let (mut reader, mut writer) = split(client);

    let handle1 = tokio::spawn(async move {
        let mut i = 0;
        loop {
            println!("SEND: PING");
            writer
                .write_all(format!("ping{i}").as_bytes())
                .await
                .expect("Unable to write message to client");
            i += 1;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });

    let handle2 = tokio::spawn(async move {
        let mut i = 0;
        let mut buf = [0u8; 10];
        loop {
            let n = reader
                .read(&mut buf[..])
                .await
                .expect("Unable to read buffer");
            if std::str::from_utf8(&buf[..n]).unwrap() == format!("pong{i}") {
                println!("RECEIVED: PONG");
            } else {
                panic!();
            }
            i += 1;
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    });
    let (r1, r2) = futures::join!(handle1, handle2);
    r1.unwrap();
    r2.unwrap();
}
