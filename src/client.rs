use crate::io;
use std::io::{Read, Cursor};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("invalid server status")]
    InvalidServerStatus,
    #[error("invalid response content type, expected {0}, got {1}")]
    InvalidContentType(&'static str, String),
    #[error(transparent)]
    RequestError(#[from] ureq::Error),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    Utf8Error(#[from] std::string::FromUtf8Error),
}

pub struct Client {
    url: String,
    client: ureq::Agent,
}

impl Client {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            client: ureq::AgentBuilder::new()
                .user_agent("anni-fetch 0.1.0")
                .build(),
        }
    }

    pub fn handshake(&mut self) -> Result<PktIter, ClientError> {
        let reader = self.client
            .get(&format!("{}/info/refs?service=git-upload-pack", &self.url))
            .set("Git-Protocol", "version=2")
            .call()?
            .into_reader();
        Ok(PktIter::new(reader))
    }

    /// Use [Client::request] instead
    #[deprecated]
    pub fn command(&self, command: &str, capabilities: Option<&[(&str, Option<&[&str]>)]>, arguments: &[&str]) -> Result<impl Read + Send, ClientError> {
        let out = Vec::new();
        let mut cursor = std::io::Cursor::new(out);
        io::write_pktline(&mut cursor, &format!("command={}", command))?;
        io::write_pktline(&mut cursor, "object-format=sha1")?;
        io::write_pktline(&mut cursor, "agent=git/2.28.0")?;

        if let Some(capabilities) = capabilities {
            for (k, v) in capabilities {
                if let Some(v) = v {
                    io::write_pktline(&mut cursor, &format!("{}={}", k, v.join(" ")))?;
                } else {
                    io::write_pktline(&mut cursor, k)?;
                }
            }
        }
        io::write_packet(&mut cursor, 1)?;

        for arg in arguments.iter() {
            io::write_pktline(&mut cursor, arg)?;
        }
        io::write_packet(&mut cursor, 0)?;

        Ok(self.client
            .post(&format!("{}/git-upload-pack", &self.url))
            .set("Git-Protocol", "version=2")
            .set("Content-Type", "application/x-git-upload-pack-request")
            .set("Accept", "application/x-git-upload-pack-result")
            .send_bytes(&cursor.into_inner())?
            .into_reader())
    }

    pub fn request(&self, body: Vec<u8>) -> Result<PktIter, ClientError> {
        let response = self.client
            .post(&format!("{}/git-upload-pack", &self.url))
            .set("Git-Protocol", "version=2")
            .set("Content-Type", "application/x-git-upload-pack-request")
            .set("Accept", "application/x-git-upload-pack-result")
            .send_bytes(&body)?;
        if response.status() != 200 {
            return Err(ClientError::InvalidServerStatus);
        } else if let None = response.header("Content-Type") {
            return Err(ClientError::InvalidContentType("application/x-git-upload-pack-result", "Nothing".to_owned()));
        } else if let Some(v) = response.header("Content-Type") {
            if v != "application/x-git-upload-pack-result" {
                return Err(ClientError::InvalidContentType("application/x-git-upload-pack-result", v.to_owned()));
            }
        }
        let reader = response.into_reader();
        Ok(PktIter::new(reader))
    }

    pub fn ls_ref(&self, prefix: &str) -> Result<String, ClientError> {
        let mut result = self.command(
            "ls-refs",
            None,
            &[
                "peel",
                "symrefs",
                &format!("ref-prefix {}", prefix),
            ],
        )?;
        let (mut result, _) = io::read_pktline(&mut result)?;
        result.truncate(40);
        Ok(String::from_utf8(result)?)
    }

    /// Use [RequestBuilder::want] with [Client::ls_ref] instead
    #[deprecated]
    pub fn want_ref(&self, prefix: &str) -> Result<String, ClientError> {
        Ok(format!("want {}", self.ls_ref(prefix)?))
    }
}

pub struct RequestBuilder {
    inner: Cursor<Vec<u8>>,
    delimeter_written: bool,
    flush_written: bool,
}

impl RequestBuilder {
    pub fn new(auto_packet: bool) -> Self {
        let mut inner = Default::default();
        io::write_pktline(&mut inner, "object-format=sha1").unwrap();
        io::write_pktline(&mut inner, "agent=git/2.28.0").unwrap();
        Self {
            inner,
            delimeter_written: auto_packet,
            flush_written: auto_packet,
        }
    }

    pub fn command(mut self, command: &str) -> Self {
        io::write_pktline(&mut self.inner, &format!("command={}", command)).unwrap();
        self
    }

    pub fn capability(mut self, name: &str, value: &[&str]) -> Self {
        if value.len() != 0 {
            io::write_pktline(&mut self.inner, &format!("{}={}", name, value.join(" "))).unwrap();
        } else {
            io::write_pktline(&mut self.inner, name).unwrap();
        }
        self
    }

    pub fn argument(mut self, arg: &str) -> Self {
        if !self.delimeter_written {
            self = self.packet(Message::Delimeter);
            self.delimeter_written = true;
        }

        io::write_pktline(&mut self.inner, arg).unwrap();
        self
    }

    pub fn packet(mut self, packet: Message) -> Self {
        match packet {
            Message::Flush => io::write_packet(&mut self.inner, 0).unwrap(),
            Message::Delimeter => io::write_packet(&mut self.inner, 1).unwrap(),
            Message::ResponseEnd => io::write_packet(&mut self.inner, 2).unwrap(),
            _ => panic!("invalid packet type")
        }
        self
    }

    pub fn want(mut self, hash: &str) -> Self {
        self.argument(&format!("want {}", hash))
    }

    pub fn have(mut self, hash: &str) -> Self {
        self.argument(&format!("have {}", hash))
    }

    pub fn build(mut self) -> Vec<u8> {
        if !self.flush_written {
            self = self.packet(Message::Flush);
            self.flush_written = true;
        }
        self.inner.into_inner()
    }
}

/// Message abstracts the type of information you may receive from a Git server.
#[derive(Debug, PartialEq)]
pub enum Message {
    Normal(Vec<u8>),
    /// 0000 Flush Packet(flush-pkt)
    ///
    /// Indicates the end of a message
    Flush,
    /// 0001 Delimeter Packet(delim-pkt)
    ///
    /// Separates sections of a message
    Delimeter,
    /// 0002 Response End Packet(response-end-pkg)
    ///
    /// Indicates the end of a response for stateless connections
    ResponseEnd,
    /// Received when data is `packfile\n`
    ///
    /// After this message, only `Pack.+` messages would be sent
    ///
    /// There is a byte at the beginning of all `Pack.+` messages except PackStart
    /// The stream code can be one of:
    /// 1 - pack data
    /// 2 - progress messages
    /// 3 - fatal error message just before stream aborts
    PackStart,
    /// Received after `Message::PackStart` when stream code is 1
    ///
    /// Data of PACK file
    PackData(Vec<u8>),
    /// Received after `Message::PackStart` when stream code is 2
    ///
    /// Progress messages of the transfer
    PackProgress(String),
    /// Received after `Message::PackStart` when stream code is 3
    ///
    /// Fatal error message
    PackError(String),
}

pub struct PktIter {
    inner: Box<dyn Read + Send>,
    is_data: bool,
}

impl PktIter {
    pub fn new(reader: impl Read + Send + 'static) -> Self {
        Self {
            inner: Box::new(reader),
            is_data: false,
        }
    }
}

impl Iterator for PktIter {
    type Item = Message;

    fn next(&mut self) -> Option<Self::Item> {
        let (mut data, len) = io::read_pktline(&mut self.inner).unwrap();
        if len == 0 && data.len() == 0 {
            None
        } else if len > 0 && self.is_data {
            match data[0] {
                1 => {
                    // pack data
                    data.remove(0);
                    Some(Message::PackData(data))
                }
                2 => {
                    // progress message
                    Some(Message::PackProgress(String::from_utf8_lossy(&data[1..]).trim().to_owned()))
                }
                3 => {
                    // fatal error
                    Some(Message::PackError(String::from_utf8_lossy(&data[1..]).trim().to_owned()))
                }
                _ => unreachable!(),
            }
        } else if data == b"packfile\n" {
            self.is_data = true;
            Some(Message::PackStart)
        } else {
            Some(match len {
                0 => Message::Flush,
                1 => Message::Delimeter,
                2 => Message::ResponseEnd,
                _ => Message::Normal(data),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Client, Pack};
    use crate::io::read_pktline;
    use crate::client::Message::*;
    use std::io::Cursor;
    use crate::client::RequestBuilder;

    #[test]
    fn test_handshake() {
        let v: Vec<_> = Client::new("https://github.com/project-anni/repo.git").handshake().unwrap().collect();
        assert_eq!(v, vec![
            Normal(b"# service=git-upload-pack\n".to_vec()),
            Flush,
            Normal(b"version 2\n".to_vec()),
            Normal(b"agent=git/github-ga3f34e80fa9a\n".to_vec()),
            Normal(b"ls-refs\n".to_vec()),
            Normal(b"fetch=shallow filter\n".to_vec()),
            Normal(b"server-option\n".to_vec()),
            Normal(b"object-format=sha1\n".to_vec()),
            Flush,
        ]);
    }

    #[test]
    fn test_ls_ref() {
        let hash = Client::new("https://github.com/project-anni/anni-fetch.git")
            .ls_ref("refs/tags")
            .unwrap();
        assert_eq!(hash, "9192b5e5f2941fd76aa5a08043dc8aa6a31831a2");
    }

    #[test]
    fn test_fetch() {
        let client = Client::new("https://github.com/project-anni/repo.git");
        let mut c = client.command("fetch", None, &[
            "thin-pack",
            "ofs-delta",
            "deepen 1",
            &client.want_ref("HEAD").expect("failed to get sha1 of HEAD"),
            "done"
        ]).unwrap();
        let mut is_data = false;
        loop {
            let (data, len) = read_pktline(&mut c).unwrap();
            if len == 0 && data.len() == 0 {
                break;
            } else if len > 0 && is_data {
                match data[0] {
                    1 => {
                        // pack data
                        println!("pack data");
                    }
                    2 => {
                        // progress message
                        println!("{}", String::from_utf8_lossy(&data[1..]).trim());
                    }
                    3 => {
                        // fatal error
                        eprintln!("{}", String::from_utf8_lossy(&data[1..]).trim());
                    }
                    _ => unreachable!(),
                }
                continue;
            } else if data == b"packfile\n" {
                is_data = true;
                continue;
            }
            println!("{}", String::from_utf8_lossy(&data).trim());
        }
    }

    #[test]
    fn test_fetch_iter() {
        let client = Client::new("https://github.com/project-anni/repo.git");
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
        let mut pack = Vec::new();
        for msg in iter {
            match msg {
                PackData(mut d) => pack.append(&mut d),
                _ => {}
            }
        }
        let mut cursor = Cursor::new(pack);
        Pack::from_reader(&mut cursor).expect("invalid pack file");
    }
}
