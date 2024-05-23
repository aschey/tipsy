# tipsy

[![crates.io](https://img.shields.io/crates/v/tipsy.svg?logo=rust)](https://crates.io/crates/tipsy)
[![docs.rs](https://img.shields.io/docsrs/tipsy?logo=rust)](https://docs.rs/tipsy)
[![Dependency Status](https://deps.rs/repo/github/aschey/tipsy/status.svg?style=flat-square)](https://deps.rs/repo/github/aschey/tipsy)
![license](https://img.shields.io/badge/License-MIT%20or%20Apache%202-green.svg)
[![CI](https://github.com/aschey/tipsy/actions/workflows/ci.yml/badge.svg)](https://github.com/aschey/tipsy/actions/workflows/ci.yml)
![GitHub repo size](https://img.shields.io/github/repo-size/aschey/tipsy)
![Lines of Code](https://aschey.tech/tokei/github/aschey/tipsy)

This is a fork of [parity-tokio-ipc](https://github.com/paritytech/parity-tokio-ipc).

[tipsy](https://github.com/aschey/tipsy) is a library for cross-platform async IPC using Tokio.
It utilizes unix sockets on UNIX (via [`tokio::net::UnixStream`](https://docs.rs/tokio/latest/tokio/net/struct.UnixStream.html))
and named pipes on windows (via [`tokio::net::windows::named_pipe`](https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/index.html)).

## Server

```rust,no_run
use tipsy::{Endpoint, OnConflict, ServerId};
use futures::stream::StreamExt;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    Endpoint::new(ServerId("my-server"), OnConflict::Overwrite)?
        .incoming()?
        .for_each(|conn| async {
            match conn {
                Ok(stream) => println!("Got connection!"),
                Err(e) => eprintln!("Error when receiving connection: {:?}", e),
            }
        });
    Ok(())
}
```

## Client

```rust,no_run
use tipsy::{Endpoint, ServerId};
use tokio::io::AsyncWriteExt;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut client = Endpoint::connect(ServerId("my-server")).await?;
    client.write_all(b"ping").await?;
    Ok(())
}
```

## Examples

See [examples](https://github.com/aschey/tipsy/tree/main/examples).

## Supported Rust Versions

The MSRV is currently `1.75.0`.
