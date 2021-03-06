use std::io::{Read, Seek, SeekFrom};
use miniz_oxide::{DataFormat, MZFlush};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::stream::{InflateState, MinReset};
use thiserror::Error;
use sha1::Digest;
use std::collections::HashMap;
use crate::io::{token, u32_be, u8};
use std::convert::TryInto;
use crate::utils::git_sha1;

const INPUT_BUFFER_SIZE: usize = 8 * 1024;
const OUTPUT_BUFFER_SIZE: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum UnpackError {
    #[error("invalid object type")]
    InvalidObjectType,
    #[error("invalid TINFL status")]
    InvalidTINFLStatus(TINFLStatus),
    #[error("invalid hash")]
    InvalidHash,
    #[error(transparent)]
    IOError(#[from] std::io::Error),
}

/// Read git variable integer and extract (object_type, length, bytes_used).
fn vint_from_reader<R: Read>(reader: &mut R) -> std::io::Result<(u8, usize, usize)> {
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
fn ofs_from_reader<R: Read>(reader: &mut R) -> std::io::Result<(usize, usize)> {
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
    pub objects: HashMap<[u8; 20], Object>,
    offsets: HashMap<usize, [u8; 20]>,
    pub sha1: [u8; 20],
}

#[derive(Debug, PartialEq)]
pub struct Object {
    pub object_type: ObjectType,
    pub data: Vec<u8>,
    pub compressed_length: usize,
    pub offset: usize,
}

#[derive(Debug, PartialEq)]
pub enum ObjectType {
    Commit,
    Tree,
    Blob,
    Tag,
    OfsDelta(usize),
    RefDelta([u8; 20]),
}

impl Pack {
    pub fn offset(&self, offset: usize) -> Option<&Object> {
        if let Some(hash) = self.offsets.get(&offset) {
            self.objects.get(hash)
        } else {
            None
        }
    }

    pub fn from_reader<R: Read + Seek>(reader: &mut R) -> std::result::Result<Self, UnpackError> {
        token(reader, b"PACK")?;
        let version = u32_be(reader)?;
        let objects = u32_be(reader)?;

        let mut offset = 12;
        let mut result = HashMap::new();
        let mut offsets = HashMap::new();

        let mut state = InflateState::new_boxed(DataFormat::Zlib);
        let mut input_buf = vec![0u8; INPUT_BUFFER_SIZE];
        let mut output_buf = vec![0u8; OUTPUT_BUFFER_SIZE];

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
                7 => {
                    let mut data = [0u8; 20];
                    reader.read_exact(&mut data)?;
                    object_size += 20;
                    RefDelta(data)
                }
                _ => return Err(UnpackError::InvalidObjectType),
            };

            let mut compressed_length = 0;
            let mut data = Vec::with_capacity(decompressed_length);
            loop {
                let bytes_available = reader.read(&mut input_buf)?;

                let (consumed, backseek, _) = Pack::extract_from(&mut state, bytes_available, &input_buf, &mut output_buf);
                compressed_length += consumed;
                data.append(&mut output_buf);
                reader.seek(SeekFrom::Current(backseek))?;

                input_buf.resize(2048, 0);
                output_buf.resize(4096, 0);
                match state.last_status() {
                    // Need more input
                    // Next turn, provide more input
                    TINFLStatus::NeedsMoreInput => {
                        continue;
                    }
                    // Need more output
                    // Output buffer of state is full, loop to pump them out
                    TINFLStatus::HasMoreOutput => {
                        loop {
                            let (_, _, produced) = Pack::extract_from(&mut state, 0, &[], &mut output_buf);
                            data.append(&mut output_buf);
                            output_buf.resize(4096, 0);
                            if produced < OUTPUT_BUFFER_SIZE {
                                break;
                            }
                        }
                        continue;
                    }
                    // Done
                    // Decode finished, but data may still in buffer of `state`
                    // Need to pump them out first
                    TINFLStatus::Done => {
                        while data.len() < decompressed_length {
                            Pack::extract_from(&mut state, 0, &[], &mut output_buf);
                            data.append(&mut output_buf);
                            output_buf.resize(4096, 0);
                        }
                        assert_eq!(data.len(), decompressed_length, "data length larget than expected decompressed length");
                        state.reset_as(MinReset);
                        break;
                    }
                    s => return Err(UnpackError::InvalidTINFLStatus(s)),
                }
            }
            object_size += compressed_length;

            let hash = git_sha1(match object_type {
                Commit => "commit",
                Tree => "tree",
                Blob => "blob",
                Tag => "tag",
                OfsDelta(_) => "o", // TODO
                RefDelta(_) => "r" // TODO
            }, &data);

            let object = Object {
                object_type,
                data,
                compressed_length,
                offset,
            };
            offsets.insert(offset, hash);
            result.insert(hash, object);
            offset += object_size;
        }

        // final sha1
        let mut hasher = sha1::Sha1::new();
        reader.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut reader.take(offset as u64), &mut hasher)?;
        let hash_result = hasher.finalize();
        let hash_result: [u8; 20] = hash_result.try_into().unwrap();
        let mut checksum = [0u8; 20];
        reader.read_exact(&mut checksum)?;
        if hash_result != checksum {
            return Err(UnpackError::InvalidHash);
        }

        // bypass EOF check for now
        // assert_eq!(std::io::copy(&mut reader.take(1), &mut input)?, 0);

        Ok(Self {
            version,
            objects: result,
            offsets,
            sha1: checksum,
        })
    }

    fn extract_from(mut state: &mut Box<InflateState>, bytes_available: usize, input_buf: &[u8], mut output_buf: &mut Vec<u8>) -> (usize, i64, usize) {
        let r = miniz_oxide::inflate::stream::inflate(
            &mut state,
            &input_buf[..bytes_available],
            &mut output_buf,
            MZFlush::Partial,
        );
        let consumed = r.bytes_consumed;
        let backseek = (consumed as i64) - (bytes_available as i64);
        let produced = r.bytes_written;
        if produced != output_buf.len() {
            output_buf.truncate(produced);
        }
        (consumed, backseek, produced)
    }
}

#[cfg(test)]
mod tests {
    use crate::pack::{vint_from_reader, Object, ObjectType};
    use crate::{Pack, Client};
    use std::io::Cursor;
    use crate::client::{RequestBuilder, Message};

    #[test]
    fn test_vint() {
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
        assert_eq!(_pack.version, 2);
        assert_eq!(_pack.offset(12).unwrap(), &Object {
            object_type: ObjectType::Commit,
            data: br"tree 90d83dbf6a598d66405eb0b4baad14990d0f2755
author yesterday17 <mmf@mmf.moe> 1615876429 +0800
committer yesterday17 <mmf@mmf.moe> 1615876429 +0800

Initial commit
".to_vec(),
            compressed_length: 117,
            offset: 12,
        });

        assert_eq!(_pack.offset(131).unwrap().object_type, ObjectType::Tree);
        assert!(_pack.offset(131).unwrap().data.starts_with(b"100644 README.md"));
        assert_eq!(_pack.offset(131).unwrap().compressed_length, 46);
        assert_eq!(_pack.offset(131).unwrap().offset, 131);

        assert_eq!(_pack.offset(179).unwrap(), &Object {
            object_type: ObjectType::Blob,
            data: br"# Test

".to_vec(),
            compressed_length: 16,
            offset: 179,
        });

        assert_eq!(_pack.sha1, [79, 16, 208, 2, 37, 46, 7, 195, 175, 219, 45, 204, 10, 184, 141, 54, 232, 171, 74, 38]);
    }

    #[test]
    fn test_ref_delta() {
        let cli = Client::new("https://github.com/project-anni/repo.git");
        let iter = cli.request(
            RequestBuilder::new(true)
                .command("fetch")
                .argument("thin-pack")
                .argument("ofs-delta")
                .argument("deepen 1")
                .want(&cli.ls_ref("HEAD").unwrap())
                .have("da32dc7b28d73b67dcbb894daf862538615d7765")
                .build()
        ).unwrap();
        let mut p = Vec::new();
        for msg in iter {
            match msg {
                Message::PackData(mut data) => p.append(&mut data),
                _ => {}
            }
        }
        Pack::from_reader(&mut Cursor::new(p)).unwrap();
    }
}