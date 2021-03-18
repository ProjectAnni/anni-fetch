# anni-fetch

[![crates.io](https://img.shields.io/crates/v/anni-fetch.svg)](https://crates.io/crates/anni-fetch)
[![API](https://docs.rs/anni-fetch/badge.svg)](https://docs.rs/anni-fetch)

A library focuses on what git fetch does.

## Example

```rust
use anni_fetch::client::Message::PackData;
use anni_fetch::{Pack, Client};
use std::io::Cursor;
use anni_fetch::client::RequestBuilder;

fn main() {
    // create client
    let client = Client::new("https://github.com/project-anni/repo.git");

    // request and get message iterator
    let iter = client.request(
        RequestBuilder::new(true)
            .command("fetch")
            .argument("thin-pack")
            .argument("ofs-delta")
            .argument("deepen 1")
            .want(&client.ls_ref("HEAD").expect("failed to get sha1 of HEAD"))
            .argument("done")
            .build()
    ).unwrap();

    // prepare buffer for pack
    let mut pack = Vec::new();
    for msg in iter {
        match msg {
            // receive and insert into pack
            PackData(mut d) => pack.append(&mut d),
            _ => {}
        }
    }
    let mut cursor = Cursor::new(pack);

    // read pack
    Pack::from_reader(&mut cursor).expect("invalid pack file");
}
```
