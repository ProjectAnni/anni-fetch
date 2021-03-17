use std::io::{Read, Seek, SeekFrom};
use anni_utils::decode::*;
use miniz_oxide::{DataFormat, MZFlush};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::stream::{InflateState, MinReset};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UnpackError {
    #[error("invalid object type")]
    InvalidObjectType,
    #[error("invalid TINFL status")]
    InvalidTINFLStatus(TINFLStatus),
    #[error(transparent)]
    DecodeError(#[from] DecodeError),
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

// TODO: return how many bytes consumed
fn vint_from_reader<R: Read>(reader: &mut R) -> std::result::Result<(u8, usize), DecodeError> {
    let mut len = 0usize;
    let mut object_type = 0u8;
    let mut i = 0;
    loop {
        // read a byte
        let n = u8(reader)?;

        // if object type is not set, it should be the first byte
        let (num, shift_inc) = if object_type == 0 {
            // set object type
            object_type = (n & 0b01110000) >> 4;
            // the num is the latter 4 bits
            (n & 0b00001111, 4)
        } else {
            // not the first byte
            // the num is the latter 7 bits
            (n & 0b01111111, 7)
        };
        // shl to add number
        len |= (num as usize) << i;
        i += shift_inc;

        // the end of VInt when MSF is 0
        if n & 0b10000000 == 0 {
            break;
        }
    }
    Ok((object_type, len))
}

// TODO: return how many bytes consumed
fn ofs_from_reader<R: Read>(reader: &mut R) -> std::result::Result<usize, DecodeError> {
    let mut n = u8(reader)?;
    let mut distance = n as usize & 0b01111111;
    while n & 0b10000000 != 0 {
        n = u8(reader)?;
        distance += 1;
        distance = (distance << 7) + (n & 0b01111111) as usize;
    }
    Ok(distance)
}

// TODO: offset
pub struct Pack {
    pub version: u32,
    pub objects: Vec<Object>,
    pub sha1: Vec<u8>,
}

pub struct Object {
    pub object_type: ObjectType,
    pub data: Vec<u8>,
    pub length_uncompressed: usize,
}

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

        let mut result = Vec::with_capacity(objects as usize);

        // Shareable data
        let mut state = InflateState::new(DataFormat::Zlib);
        let mut output_data = Vec::new();
        let mut input = Vec::with_capacity(2048);

        for _ in 0..objects {
            use crate::pack::ObjectType::*;
            let (object_type, len) = vint_from_reader(reader)?;
            let object_type = match object_type {
                1 => Commit,
                2 => Tree,
                3 => Blob,
                4 => Tag,
                6 => OfsDelta(ofs_from_reader(reader)?),
                7 => RefDelta(Vec::new()), // TODO
                _ => return Err(UnpackError::InvalidObjectType),
            };

            let mut exact_size;
            let mut offset = 0;
            loop {
                offset += std::io::copy(&mut reader.take(len as u64), &mut input)?;

                let r = miniz_oxide::inflate::stream::inflate(
                    &mut state,
                    &input,
                    &mut output_data,
                    MZFlush::Partial,
                );
                exact_size = r.bytes_consumed;
                match state.last_status() {
                    TINFLStatus::Done => {
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
            reader.seek(SeekFrom::Current(-(offset as i64 - exact_size as i64)))?;

            let data = match miniz_oxide::inflate::decompress_to_vec_zlib(&input) {
                Ok(data) => data,
                Err(s) => return Err(UnpackError::InvalidTINFLStatus(s)),
            };
            input.clear();

            let object = Object {
                object_type,
                data,
                length_uncompressed: len,
            };
            result.push(object);
        }

        // final sha1
        let sha1 = take(reader, 20)?;
        // EOF
        assert_eq!(std::io::copy(&mut reader.take(1), &mut input)?, 0);

        Ok(Self {
            version,
            objects: result,
            sha1,
        })
    }
}

#[test]
fn test_vint() {
    use std::io::Cursor;
    assert_eq!(vint_from_reader(&mut Cursor::new(&[0b00101111])).unwrap(), (0b010, 0b1111));
    assert_eq!(vint_from_reader(&mut Cursor::new(&[0b10010101, 0b00001010])).unwrap(),
               (0b001, 0b0101 + (0b1010 << 4))
    );
    assert_eq!(vint_from_reader(&mut Cursor::new(
        &[0b10101111, 0b10101100, 0b10010010, 0b01110101])).unwrap(),
               (0b010, 0b1111 + (0b0101100 << 4) + (0b0010010 << 11) + (0b1110101 << 18)),
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
    Pack::from_reader(&mut std::io::Cursor::new(data)).expect("parse failed");
}

#[test]
fn test_pack_large() {
    let mut file = std::fs::File::open("/tmp/test").expect("open failed");
    Pack::from_reader(&mut file).expect("parse_failed");
}
