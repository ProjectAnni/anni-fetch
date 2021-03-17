//! anni-fetch
//!
//! `anni-fetch` is interact with git server and fetch pack files from it.
//! It implemented git v2 protocol and PACK file uncompression, which is used in git fetch procedure.
//!
//! # Example
//!
//! ```
//! use anni_fetch::Client;
//!
//! let client = Client::new("https://github.com/project-anni/repo.git");
//! let iter = client.request("fetch", None, &[
//!     "thin-pack",
//!     "ofs-delta",
//!     "deepen 1",
//!     &client.want_ref("HEAD").expect("failed to get sha1 of HEAD"),
//!     "done"
//! ]).unwrap();
//! for msg in iter {
//!     println!("{:?}", msg);
//! }
//! ```
//!
//! You can use `match` to filter the type of message you want.
//! For example, you can just receive `Message::PackData` and
//! write the content to a `pak` file.

pub mod io;
pub mod pack;
pub mod client;

pub use client::Client;
pub use pack::Pack;
