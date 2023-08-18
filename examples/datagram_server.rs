use std::time::Duration;

use futures::StreamExt as _;
use parity_tokio_ipc::{
    ConnectionId, ConnectionType, Endpoint, IpcEndpoint, IpcSecurity, SecurityAttributes,
};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};

async fn run_server(path: String) {
    let mut endpoint = Endpoint::new(ConnectionId(path), ConnectionType::Datagram);
    endpoint.set_security_attributes(SecurityAttributes::allow_everyone_create().unwrap());

    let incoming = endpoint.incoming().expect("failed to open new socket");
    futures::pin_mut!(incoming);

    while let Some(result) = incoming.next().await {
        match result {
            Ok(stream) => {
                let (mut reader, mut writer) = split(stream);

                tokio::spawn(async move {
                    let mut i = 0;
                    let mut buf = [0u8; 10];
                    loop {
                        let n = reader.read(&mut buf).await.unwrap();
                        if std::str::from_utf8(&buf[..n]).unwrap() == format!("ping{i}") {
                            println!("RECEIVED: PING");
                        } else {
                            panic!()
                        }
                        i += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
                });
                tokio::spawn(async move {
                    let mut i = 0;
                    loop {
                        writer
                            .write_all(format!("pong{i}").as_bytes())
                            .await
                            .expect("unable to write to socket");
                        i += 1;
                        println!("SEND: PONG");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                });
            }
            _ => unreachable!("ideally"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Run it with server path as argument");
    run_server(path).await
}
