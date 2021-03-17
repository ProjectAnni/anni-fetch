/// https://git-scm.com/docs/protocol-common

use std::io::{Read, Write};
use std::convert::TryInto;

pub(crate) fn take_sized<R: Read>(reader: &mut R, len: usize) -> std::io::Result<(Vec<u8>, u64)> {
    let mut r = Vec::with_capacity(len);
    let got = std::io::copy(&mut reader.take(len as u64), &mut r)?;
    Ok((r, got))
}

pub(crate) fn token<R: Read>(reader: &mut R, token: &[u8]) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let (got, read) = take_sized(reader, token.len())?;
    if read != token.len() as u64 {
        Err(Error::new(ErrorKind::InvalidInput, "more data needed"))
    } else if got[..] == token[..] {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidData, "token mismatch"))
    }
}

#[inline]
pub(crate) fn u8<R: Read>(reader: &mut R) -> std::io::Result<u8> {
    let mut buf = [0; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0])
}

#[inline]
pub(crate) fn u32_be<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut buf = [0; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf[..].try_into().unwrap()))
}

fn read_len<R: Read>(reader: &mut R) -> std::io::Result<usize> {
    let (next, got) = take_sized(reader, 4)?;
    if got != 4 {
        // EOF, return 0x10000(>0xffff)
        Ok(0x10000)
    } else {
        let str = String::from_utf8_lossy(&next);
        let len = usize::from_str_radix(str.as_ref(), 16).unwrap();
        Ok(len)
    }
}

/// Read pkgline from a reader
pub fn read_pktline<R: Read>(reader: &mut R) -> std::io::Result<(Vec<u8>, usize)> {
    let mut len = read_len(reader)?;
    let data = if len == 0x10000 {
        len = 0;
        Vec::new()
    } else if len >= 4 {
        let (data, _) = take_sized(reader, len - 4)?;
        data
    } else {
        format!("{:04x}", len).as_bytes().to_vec()
    };
    Ok((data, len))
}

///  When the grammar indicate PKT-LINE(...), unless otherwise noted the usual pkt-line LF rules apply:
///  the sender SHOULD include a LF, but the receiver MUST NOT complain if it is not present.
pub fn write_pktline<W: Write>(writer: &mut W, data: &str) -> std::io::Result<()> {
    writer.write(format!("{:04x}", data.len() + 1 + 4).as_bytes())?;
    writer.write(data.as_bytes())?;
    writer.write(b"\n")?;
    Ok(())
}

/// Write pkt line without the padding LF character
pub fn write_pktline_nolf<W: Write>(writer: &mut W, data: &str) -> std::io::Result<()> {
    writer.write(format!("{:04x}", data.len() + 4).as_bytes())?;
    writer.write(data.as_bytes())?;
    Ok(())
}

/// 0000 Flush Packet
/// 0001 Delimiter Packet
/// 0002 Response End Packet
pub(crate) fn write_packet<W: Write>(writer: &mut W, data: u8) -> std::io::Result<()> {
    writer.write(format!("{:04x}", data).as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::io::{write_pktline, read_pktline, take_sized, token, u8, u32_be, read_len, write_pktline_nolf, write_packet};
    use std::io::{Read, Cursor};

    #[test]
    fn test_take_sized() {
        let v = [1, 2, 3];
        let mut c = Cursor::new(v);
        let (v, got) = take_sized(&mut c, 2).unwrap();
        assert_eq!(v, vec![1, 2]);
        assert_eq!(got, 2);

        let (v, got) = take_sized(&mut c, 2).unwrap();
        assert_eq!(v, vec![3]);
        assert_eq!(got, 1);
    }

    #[test]
    fn test_token() {
        let t = b"fLaCtest";
        let mut c = Cursor::new(t);
        token(&mut c, b"fLaC").expect("fLaC token error");
        token(&mut c, b"2333").expect_err("token not match");
    }

    #[test]
    fn test_u8() {
        let v = [1, 2, 3];
        let mut c = Cursor::new(v);
        assert_eq!(u8(&mut c).unwrap(), 1);
        assert_eq!(u8(&mut c).unwrap(), 2);
        assert_eq!(u8(&mut c).unwrap(), 3);
        u8(&mut c).expect_err("fn should have no byte to read");
    }

    #[test]
    fn test_u32_be() {
        let v = [0xaa, 0xbb, 0xcc, 0xdd];
        let mut c = Cursor::new(v);
        assert_eq!(u32_be(&mut c).unwrap(), 0xaabbccdd);
    }

    #[test]
    fn test_read_len() {
        let v = b"00100000";
        let mut c = Cursor::new(v);
        assert_eq!(read_len(&mut c).unwrap(), 0x10);
        assert_eq!(read_len(&mut c).unwrap(), 0);
        assert_eq!(read_len(&mut c).unwrap(), 0x10000);
    }

    #[test]
    fn test_pktline_read() {
        let data = [
            0x30, 0x30, 0x31, 0x65, 0x23, 0x20, 0x73, 0x65, 0x72, 0x76, 0x69, 0x63, 0x65, 0x3d, 0x67, 0x69,
            0x74, 0x2d, 0x75, 0x70, 0x6c, 0x6f, 0x61, 0x64, 0x2d, 0x70, 0x61, 0x63, 0x6b, 0x0a, 0x30, 0x30,
            0x30, 0x30, 0x30, 0x30, 0x30, 0x65, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x32, 0x0a,
            0x30, 0x30, 0x32, 0x33, 0x61, 0x67, 0x65, 0x6e, 0x74, 0x3d, 0x67, 0x69, 0x74, 0x2f, 0x67, 0x69,
            0x74, 0x68, 0x75, 0x62, 0x2d, 0x67, 0x31, 0x38, 0x63, 0x33, 0x31, 0x39, 0x39, 0x33, 0x39, 0x34,
            0x61, 0x63, 0x0a, 0x30, 0x30, 0x30, 0x63, 0x6c, 0x73, 0x2d, 0x72, 0x65, 0x66, 0x73, 0x0a, 0x30,
            0x30, 0x31, 0x39, 0x66, 0x65, 0x74, 0x63, 0x68, 0x3d, 0x73, 0x68, 0x61, 0x6c, 0x6c, 0x6f, 0x77,
            0x20, 0x66, 0x69, 0x6c, 0x74, 0x65, 0x72, 0x0a, 0x30, 0x30, 0x31, 0x32, 0x73, 0x65, 0x72, 0x76,
            0x65, 0x72, 0x2d, 0x6f, 0x70, 0x74, 0x69, 0x6f, 0x6e, 0x0a, 0x30, 0x30, 0x31, 0x37, 0x6f, 0x62,
            0x6a, 0x65, 0x63, 0x74, 0x2d, 0x66, 0x6f, 0x72, 0x6d, 0x61, 0x74, 0x3d, 0x73, 0x68, 0x61, 0x31,
            0x0a, 0x30, 0x30, 0x30, 0x30,
        ];
        let mut cursor = std::io::Cursor::new(data);
        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"# service=git-upload-pack\n");
        assert_eq!(s, 0x001e);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"0000");
        assert_eq!(s, 0x0000);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"version 2\n");
        assert_eq!(s, 0x000e);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"agent=git/github-g18c3199394ac\n");
        assert_eq!(s, 0x0023);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"ls-refs\n");
        assert_eq!(s, 0x000c);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"fetch=shallow filter\n");
        assert_eq!(s, 0x0019);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"server-option\n");
        assert_eq!(s, 0x0012);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"object-format=sha1\n");
        assert_eq!(s, 0x0017);

        let (r, s) = read_pktline(&mut cursor).expect("failed to read");
        assert_eq!(r, b"0000");
        assert_eq!(s, 0x0000);

        let mut r = [0; 0];
        let r = cursor.read(&mut r).expect("failed to read");
        assert_eq!(r, 0);
    }

    #[test]
    fn test_pktline_write() {
        let out = Vec::with_capacity(100);
        let mut cursor = std::io::Cursor::new(out);
        write_pktline(&mut cursor, "test").expect("failed to write pktline");
        write_pktline_nolf(&mut cursor, "another_test").expect("failed to write pktline");
        write_packet(&mut cursor, 0).expect("failed to write packet");
        assert_eq!(cursor.into_inner(), b"0009test\n0010another_test0000")
    }
}