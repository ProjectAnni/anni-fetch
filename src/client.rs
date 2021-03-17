use crate::io;
use std::io::Read;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
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

    pub fn request(&self, command: &str, capabilities: Option<&[(&str, Option<&[&str]>)]>, arguments: &[&str]) -> Result<PktIter, ClientError> {
        let reader = self.command(command, capabilities, arguments)?;
        Ok(PktIter::new(reader))
    }

    pub fn ls_ref(&self, prefix: &str) -> Result<String, ClientError> {
        let mut result = self.command("ls-refs", None, &[&format!("ref-prefix {}", prefix)])?;
        let (mut result, _) = io::read_pktline(&mut result)?;
        result.truncate(40);
        Ok(String::from_utf8(result)?)
    }

    pub fn want_ref(&self, prefix: &str) -> Result<String, ClientError> {
        Ok(format!("want {}", self.ls_ref(prefix)?))
    }
}

#[derive(Debug, PartialEq)]
pub enum Message {
    Normal(Vec<u8>),
    Flush,
    Delimeter,
    ResponseEnd,
    PackStart,
    PackData(Vec<u8>),
    PackProgress(String),
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
        let (data, len) = io::read_pktline(&mut self.inner).unwrap();
        if len == 0 && data.len() == 0 {
            None
        } else if len > 0 && self.is_data {
            match data[0] {
                1 => {
                    // pack data
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
    use crate::Client;
    use crate::io::read_pktline;
    use crate::client::Message::*;

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
    fn test_ls_refs() {
        let mut c = Client::new("https://github.com/project-anni/repo.git")
            .command("ls-refs", None, &["ref-prefix HEAD"]).unwrap();
        loop {
            let (data, len) = read_pktline(&mut c).unwrap();
            if len == 0 && data.len() == 0 {
                break;
            }
            println!("{:?}", String::from_utf8_lossy(&data));
        }
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
        let iter = client.request("fetch", None, &[
            "thin-pack",
            "ofs-delta",
            "deepen 1",
            &client.want_ref("HEAD").expect("failed to get sha1 of HEAD"),
            "done"
        ]).unwrap();
        for msg in iter {
            println!("{:?}", msg);
        }
    }
}
