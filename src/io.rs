/// https://git-scm.com/docs/protocol-common

use std::io::{Read, Write};
use anni_utils::decode::take_sized;

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

pub fn write_pktline_nolf<W: Write>(writer: &mut W, data: &str) -> std::io::Result<()> {
    writer.write(format!("{:04x}", data.len() + 4).as_bytes())?;
    writer.write(data.as_bytes())?;
    Ok(())
}

/// 0000 Flush Packet
/// 0001 Delimiter Packet
/// 0002 Response End Packet
pub fn write_packet<W: Write>(writer: &mut W, data: u8) -> std::io::Result<()> {
    writer.write(format!("{:04x}", data).as_bytes())?;
    Ok(())
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
    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"# service=git-upload-pack\n");
    assert_eq!(s, 0x001e);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"0000");
    assert_eq!(s, 0x0000);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"version 2\n");
    assert_eq!(s, 0x000e);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"agent=git/github-g18c3199394ac\n");
    assert_eq!(s, 0x0023);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"ls-refs\n");
    assert_eq!(s, 0x000c);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"fetch=shallow filter\n");
    assert_eq!(s, 0x0019);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"server-option\n");
    assert_eq!(s, 0x0012);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"object-format=sha1\n");
    assert_eq!(s, 0x0017);

    let (r, s) = read_pktline(&mut cursor).expect("read");
    assert_eq!(r, b"0000");
    assert_eq!(s, 0x0000);

    let mut r = [0; 0];
    let r = cursor.read(&mut r).expect("read");
    assert_eq!(r, 0);
}

#[test]
fn test_pktline_write() {
    let out = Vec::with_capacity(100);
    let mut cursor = std::io::Cursor::new(out);
    write_pktline(&mut cursor, "test").expect("should success");
    assert_eq!(cursor.into_inner(), b"0005test\n")
}