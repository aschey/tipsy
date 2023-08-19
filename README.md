# parity-tokio-ipc

[![CI](https://github.com/paritytech/parity-tokio-ipc/actions/workflows/ci.yml/badge.svg)](https://github.com/paritytech/parity-tokio-ipc/actions/workflows/ci.yml)
[![Documentation](https://docs.rs/parity-tokio-ipc/badge.svg)](https://docs.rs/parity-tokio-ipc)

This crate abstracts interprocess transport for UNIX/Windows.

It utilizes unix sockets on UNIX (via `tokio::net::UnixStream`) and named pipes on windows (via `tokio::net::windows::named_pipe` module).

Endpoint is transport-agnostic interface for incoming connections:

```rust,no_run
use parity_tokio_ipc::{Endpoint, IpcEndpoint};
use futures::stream::StreamExt;

let server = async move {
    Endpoint::new("path")
        .incoming()
        .expect("Couldn't set up server")
        .for_each(|conn| async {
            match conn {
                Ok(stream) => println!("Got connection!"),
                Err(e) => eprintln!("Error when receiving connection: {:?}", e),
            }
        });
};

let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
rt.block_on(server);
```

## License

`parity-tokio-ipc` is primarily distributed under the terms of both the MIT
license and the Apache License (Version 2.0), with portions covered by various
BSD-like licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.
