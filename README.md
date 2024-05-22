# tipsy

[![Dependency Status](https://deps.rs/repo/github/aschey/tipsy/status.svg?style=flat-square)](https://deps.rs/repo/github/aschey/tipsy)
![license](https://img.shields.io/badge/License-MIT%20or%20Apache%202-green.svg)
[![CI](https://github.com/aschey/tipsy/actions/workflows/ci.yml/badge.svg)](https://github.com/aschey/tipsy/actions/workflows/ci.yml)
![GitHub repo size](https://img.shields.io/github/repo-size/aschey/tipsy)
![Lines of Code](https://aschey.tech/tokei/github/aschey/tipsy)

This crate abstracts interprocess transport for UNIX/Windows.

It utilizes unix sockets on UNIX (via `tokio::net::UnixStream`) and named pipes on windows (via `tokio::net::windows::named_pipe` module).

Endpoint is transport-agnostic interface for incoming connections:

```rust,no_run
use tipsy::{Endpoint, IpcEndpoint, OnConflict, ServerId};
use futures::stream::StreamExt;

let server = async move {
    Endpoint::new(ServerId("id"), OnConflict::Overwrite)
        .unwrap()
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
