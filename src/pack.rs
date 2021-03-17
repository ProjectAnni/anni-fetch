use std::io::{Read, Seek, SeekFrom};
use anni_utils::decode::*;
use miniz_oxide::{DataFormat, MZFlush};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::stream::{InflateState, MinReset};
use thiserror::Error;
use sha1::Digest;
use std::collections::BTreeMap;

#[derive(Debug, Error)]
pub enum UnpackError {
    #[error("invalid object type")]
    InvalidObjectType,
    #[error("invalid TINFL status")]
    InvalidTINFLStatus(TINFLStatus),
    #[error("invalid hash")]
    InvalidHash,
    #[error(transparent)]
    DecodeError(#[from] DecodeError),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

/// Read git variable integer and extract (object_type, length, bytes_used).
fn vint_from_reader<R: Read>(reader: &mut R) -> std::result::Result<(u8, usize, usize), DecodeError> {
    let mut n = u8(reader)?;
    let object_type = (n >> 4) & 0b00000111;
    let mut len = (n as usize) & 0b00001111;

    let mut shift = 4;
    let mut used = 1;
    while n & 0b10000000 != 0 {
        n = u8(reader)?;
        len |= ((n as usize) & 0b01111111) << shift;
        shift += 7;
        used += 1;
    }
    Ok((object_type, len, used))
}

/// Read OFS_DELTA offset and extract (distance, bytes_used).
fn ofs_from_reader<R: Read>(reader: &mut R) -> std::result::Result<(usize, usize), DecodeError> {
    let mut n = u8(reader)?;
    let mut used = 1;
    let mut distance = n as usize & 0b01111111;
    while n & 0b10000000 != 0 {
        n = u8(reader)?;
        distance += 1;
        distance = (distance << 7) + (n & 0b01111111) as usize;
        used += 1;
    }
    Ok((distance, used))
}

#[derive(Debug)]
pub struct Pack {
    pub version: u32,
    pub objects: BTreeMap<usize, Object>,
    pub sha1: Vec<u8>,
}

#[derive(Debug)]
pub struct Object {
    pub object_type: ObjectType,
    pub data: Vec<u8>,
    pub compressed_length: usize,
    pub offset: usize,
}

#[derive(Debug)]
pub enum ObjectType {
    Commit,
    Tree,
    Blob,
    Tag,
    OfsDelta(usize),
    RefDelta(Vec<u8>),
}

impl Pack {
    pub fn from_reader<R: Read + Seek>(reader: &mut R) -> std::result::Result<Self, UnpackError> {
        token(reader, b"PACK")?;
        let version = u32_be(reader)?;
        let objects = u32_be(reader)?;

        let mut offset = 12;
        let mut result = BTreeMap::new();

        // Shareable data
        let mut state = InflateState::new(DataFormat::Zlib);
        let mut output_data = Vec::new();
        let mut input = Vec::with_capacity(2048);

        for _ in 0..objects {
            use crate::pack::ObjectType::*;
            let (object_type, decompressed_length, mut object_size) = vint_from_reader(reader)?;
            let object_type = match object_type {
                1 => Commit,
                2 => Tree,
                3 => Blob,
                4 => Tag,
                6 => {
                    let (d, u) = ofs_from_reader(reader)?;
                    object_size += u;
                    OfsDelta(d)
                }
                7 => RefDelta(Vec::new()), // TODO
                _ => return Err(UnpackError::InvalidObjectType),
            };

            let compressed_length;
            let mut data_copied = 0;
            loop {
                data_copied += std::io::copy(&mut reader.take(decompressed_length as u64), &mut input)?;

                let r = miniz_oxide::inflate::stream::inflate(
                    &mut state,
                    &input,
                    &mut output_data,
                    MZFlush::Partial,
                );
                match state.last_status() {
                    TINFLStatus::Done => {
                        compressed_length = r.bytes_consumed;
                        state.reset_as(MinReset);
                        break;
                    }
                    TINFLStatus::NeedsMoreInput => {
                        state.reset_as(MinReset);
                        continue;
                    }
                    s => return Err(UnpackError::InvalidTINFLStatus(s)),
                }
            }
            object_size += compressed_length;
            reader.seek(SeekFrom::Current(-(data_copied as i64 - compressed_length as i64)))?;

            let data = match miniz_oxide::inflate::decompress_to_vec_zlib(&input) {
                Ok(data) => data,
                Err(s) => return Err(UnpackError::InvalidTINFLStatus(s)),
            };
            input.clear();

            let object = Object {
                object_type,
                data,
                compressed_length,
                offset,
            };
            result.insert(offset, object);
            offset += object_size;
        }

        // final sha1
        let mut hasher = sha1::Sha1::new();
        reader.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut reader.take(offset as u64), &mut hasher)?;
        let hash_result = hasher.finalize();
        let checksum = take(reader, 20)?;
        if hash_result[..] != checksum[..] {
            return Err(UnpackError::InvalidHash);
        }

        // bypass EOF check for now
        // assert_eq!(std::io::copy(&mut reader.take(1), &mut input)?, 0);

        Ok(Self {
            version,
            objects: result,
            sha1: checksum,
        })
    }
}

#[test]
fn test_vint() {
    use std::io::Cursor;
    assert_eq!(vint_from_reader(&mut Cursor::new(&[0b00101111])).unwrap(), (0b010, 0b1111, 1));
    assert_eq!(vint_from_reader(&mut Cursor::new(&[0b10010101, 0b00001010])).unwrap(),
               (0b001, 0b0101 + (0b1010 << 4), 2)
    );
    assert_eq!(vint_from_reader(&mut Cursor::new(
        &[0b10101111, 0b10101100, 0b10010010, 0b01110101])).unwrap(),
               (0b010, 0b1111 + (0b0101100 << 4) + (0b0010010 << 11) + (0b1110101 << 18), 4),
    );
}

#[test]
fn test_unpack() {
    let data = [
        0x50, 0x41, 0x43, 0x4b, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03,
        0x95, 0x0a, 0x78, 0x9c, 0x95, 0x8b, 0x3b, 0x0a, 0x42, 0x31, 0x10, 0x00,
        0xfb, 0x9c, 0x62, 0x7b, 0x41, 0x36, 0xcf, 0x7c, 0x41, 0xc4, 0xd6, 0x63,
        0x6c, 0xcc, 0x06, 0x03, 0xae, 0x81, 0xb8, 0x16, 0xef, 0xf6, 0x06, 0x3c,
        0x81, 0xc5, 0x54, 0x33, 0xa3, 0x93, 0x19, 0x32, 0xd6, 0x74, 0xaa, 0xa5,
        0x05, 0xf2, 0x39, 0xd5, 0x10, 0x1c, 0x7a, 0x2e, 0x58, 0x5c, 0x21, 0xaa,
        0xd6, 0xe5, 0xa5, 0xb1, 0x6d, 0xd1, 0x7b, 0x43, 0x1f, 0x7d, 0x8c, 0x09,
        0x3b, 0xbf, 0x95, 0x67, 0xa5, 0xdd, 0x46, 0x38, 0x8b, 0xb4, 0xeb, 0xe2,
        0x28, 0x83, 0x2f, 0x60, 0x83, 0xf5, 0x29, 0x06, 0xb7, 0x65, 0x38, 0x60,
        0x42, 0x34, 0xf7, 0x21, 0xd2, 0x75, 0xd5, 0xff, 0x4c, 0xe6, 0xf6, 0xea,
        0xda, 0xe9, 0x09, 0xbf, 0xdb, 0x7c, 0x01, 0x31, 0x47, 0x31, 0xae, 0xa5,
        0x02, 0x78, 0x9c, 0x33, 0x34, 0x30, 0x30, 0x33, 0x31, 0x51, 0x08, 0x72,
        0x75, 0x74, 0xf1, 0x75, 0xd5, 0xcb, 0x4d, 0x61, 0xe8, 0xd8, 0x59, 0x1d,
        0x76, 0x3a, 0x81, 0xb7, 0x63, 0xfb, 0xb2, 0xdd, 0x53, 0x39, 0x9e, 0x31,
        0xf0, 0x9c, 0xfb, 0xbb, 0x54, 0x1a, 0x00, 0xdd, 0x01, 0x0e, 0x01, 0x38,
        0x78, 0x9c, 0x53, 0x56, 0x08, 0x49, 0x2d, 0x2e, 0xe1, 0xe2, 0x02, 0x00,
        0x09, 0x37, 0x01, 0xf8, 0x4f, 0x10, 0xd0, 0x02, 0x25, 0x2e, 0x07, 0xc3,
        0xaf, 0xdb, 0x2d, 0xcc, 0x0a, 0xb8, 0x8d, 0x36, 0xe8, 0xab, 0x4a, 0x26,
    ];
    let _pack = Pack::from_reader(&mut std::io::Cursor::new(data)).expect("parse failed");
    // println!("{:?}", pack);
}

#[test]
fn test_pack_large() {
    let mut file = std::fs::File::open("/tmp/test").expect("open failed");
    let _pack = Pack::from_reader(&mut file).expect("parse failed");
    // println!("{:?}", pack);
}
