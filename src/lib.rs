//! anni-fetch
//!
//! `anni-fetch` is written to interact with git server and fetch pack files from it.
//! It implemented git v2 protocol and PACK file uncompression, which is used in git fetch procedure.
//!
//! # Example
//!
//! ```rust
//! use anni_fetch::client::Message::PackData;
//! use anni_fetch::{Pack, Client};
//! use std::io::Cursor;
//! use anni_fetch::client::RequestBuilder;
//!
//! fn main() {
//!     let client = Client::new("https://github.com/project-anni/repo.git");
//!     let iter = client.request(
//!         RequestBuilder::new(true)
//!             .command("fetch")
//!             .argument("thin-pack")
//!             .argument("ofs-delta")
//!             .argument("deepen 1")
//!             .want(&client.ls_ref("HEAD").expect("failed to get sha1 of HEAD"))
//!             .argument("done")
//!             .build()
//!     ).unwrap();
//!     let mut pack = Vec::new();
//!     for msg in iter {
//!         match msg {
//!             PackData(mut d) => pack.append(&mut d),
//!             _ => {}
//!         }
//!     }
//!     let mut cursor = Cursor::new(pack);
//!     Pack::from_reader(&mut cursor).expect("invalid pack file");
//! }
//! ```
//!
//! You can use `match` to filter the type of message you want.
//! For example, you can just receive `Message::PackData` and
//! write the content to a `pak` file.

pub mod io;
pub mod pack;
pub mod client;
mod utils;

pub use client::Client;
pub use pack::Pack;
