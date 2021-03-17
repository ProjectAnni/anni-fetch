# anni-fetch

[![crates.io](https://img.shields.io/crates/v/anni-fetch.svg)](https://crates.io/crates/anni-fetch)
[![API](https://docs.rs/anni-fetch/badge.svg)](https://docs.rs/anni-fetch)

A library focuses on what git fetch does.

## Example

```rust
use anni_fetch::Client;

fn main() {
    // create a new client
    let client = Client::new("https://github.com/project-anni/repo.git");
    // fetch and read as an iterator
    let iter = client.request("fetch", None, &[
        "thin-pack",
        "ofs-delta",
        "deepen 1",
        &client.want_ref("HEAD").expect("failed to get sha1 of HEAD"),
        "done"
    ]).unwrap();

    // buffer for pack file
    let mut pack = Vec::new();
    for msg in iter {
        match msg {
            // only handle PackData
            PackData(mut d) => pack.append(&mut d),
            _ => {}
        }
    }
    let mut cursor = Cursor::new(pack);
    // parse pack
    let _pack = Pack::from_reader(&mut cursor).expect("invalid pack file");
}
```
