use futures::StreamExt as _;
use tipsy::{Endpoint, IpcEndpoint, IpcSecurity, OnConflict, SecurityAttributes, ServerId};
use tokio::io::{split, AsyncReadExt, AsyncWriteExt};

async fn run_server(path: String) {
    let mut endpoint = Endpoint::new(ServerId(path), OnConflict::Overwrite).unwrap();
    endpoint.set_security_attributes(SecurityAttributes::allow_everyone_create().unwrap());

    let incoming = endpoint.incoming().expect("failed to open new socket");
    futures::pin_mut!(incoming);

    while let Some(result) = incoming.next().await {
        match result {
            Ok(stream) => {
                let (mut reader, mut writer) = split(stream);

                tokio::spawn(async move {
                    loop {
                        let mut buf = [0u8; 4];

                        if reader.read_exact(&mut buf).await.is_err() {
                            println!("Closing socket");
                            break;
                        }
                        if let Ok("ping") = std::str::from_utf8(&buf[..]) {
                            println!("RECEIVED: PING");
                            writer
                                .write_all(b"pong")
                                .await
                                .expect("unable to write to socket");
                            println!("SEND: PONG");
                        }
                    }
                });
            }
            _ => unreachable!("ideally"),
        }
    }
}

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Run it with server path as argument");
    run_server(path).await
}
