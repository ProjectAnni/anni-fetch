use sha1::Digest;
use std::io::Write;
use std::convert::TryInto;

pub(crate) fn hex(input: &[u8]) -> String {
    let mut result = String::with_capacity(input.len() * 2);
    for v in input {
        result.push_str(&format!("{:02x}", v));
    }
    result
}

pub(crate) fn git_sha1(prefix: &str, input: &[u8]) -> [u8; 20] {
    let mut hasher = sha1::Sha1::new();
    hasher.write_all(prefix.as_bytes()).unwrap();
    hasher.write_all(b" ").unwrap();
    hasher.write_all(format!("{}", input.len()).as_bytes()).unwrap();
    hasher.write_all(&[0]).unwrap();
    hasher.write_all(input).unwrap();
    hasher.finalize().try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use crate::utils::{hex, git_sha1};

    #[test]
    fn test_hex() {
        assert_eq!(hex(&[0x00, 0x01, 0x10, 0x11, 0xfe, 0xef]), "00011011feef");
    }

    #[test]
    fn test_git_sha1() {
        assert_eq!(git_sha1("blob", &[]), [0xe6, 0x9d, 0xe2, 0x9b, 0xb2, 0xd1, 0xd6, 0x43, 0x4b, 0x8b, 0x29, 0xae, 0x77, 0x5a, 0xd8, 0xc2, 0xe4, 0x8c, 0x53, 0x91]);
    }
}