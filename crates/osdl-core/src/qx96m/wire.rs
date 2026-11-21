use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{self, Instant};

use super::Error;

const MAX_FRAME_BYTES: usize = 4 << 20;

/// Calls implemented by the native status reader.
#[derive(Clone, Copy)]
pub(super) enum Read {
    Connect = 0,
    Status = 3,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Request {
    #[serde(rename = "ID")]
    id: u64,
    #[serde(rename = "Type")]
    kind: u8,
    state: u8,
    result: bool,
    data: Option<String>,
    tag: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct Reply {
    #[serde(rename = "ID")]
    id: Option<u64>,
    #[serde(rename = "Type")]
    kind: u8,
    state: Option<i32>,
    pub result: Option<bool>,
    pub data: Option<String>,
}

pub(super) async fn exchange(
    link: &mut BufReader<TcpStream>,
    key: &[u8; 8],
    id: u64,
    read: Read,
    deadline: Instant,
) -> Result<Reply, Error> {
    time::timeout_at(deadline, async {
        link.get_mut().write_all(&request(key, id, read)).await?;
        let mut frame = Vec::new();
        loop {
            frame.clear();
            let mut bounded = (&mut *link).take((MAX_FRAME_BYTES + 1) as u64);
            if bounded.read_until(b'\n', &mut frame).await? == 0 {
                return Err(Error::Unavailable);
            }
            if frame.len() > MAX_FRAME_BYTES {
                return Err(Error::Protocol);
            }
            let encoded = frame.strip_suffix(b"\r\n").ok_or(Error::Protocol)?;
            if encoded.iter().all(|byte| *byte == 0) {
                continue;
            }
            let reply: Reply = decode(encoded, key)?;
            if reply.kind == read as u8 && reply.id == Some(id) {
                if reply.state.is_none() || reply.result.is_none() {
                    return Err(Error::Protocol);
                }
                return Ok(reply);
            }
        }
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

fn request(key: &[u8; 8], id: u64, read: Read) -> Vec<u8> {
    let plain = serde_json::to_vec(&Request {
        id,
        kind: read as u8,
        state: 0,
        result: true,
        data: None,
        tag: None,
    })
    .expect("fixed QX96M request is serializable");
    encode(&plain, key)
}

pub(super) fn encode(plain: &[u8], key: &[u8; 8]) -> Vec<u8> {
    let encrypted = cbc::Encryptor::<des::Des>::new(key.into(), key.into())
        .encrypt_padded_vec_mut::<Pkcs7>(plain);
    let mut frame = hex::encode_upper(encrypted).into_bytes();
    frame.extend_from_slice(b"\r\n");
    frame
}

fn decode<T: for<'de> Deserialize<'de>>(encoded: &[u8], key: &[u8; 8]) -> Result<T, Error> {
    let mut bytes = hex::decode(encoded).map_err(|_| Error::Protocol)?;
    let plain = cbc::Decryptor::<des::Des>::new(key.into(), key.into())
        .decrypt_padded_mut::<Pkcs7>(&mut bytes)
        .map_err(|_| Error::Protocol)?;
    let text = encoding_rs::GB18030
        .decode_without_bom_handling_and_without_replacement(plain)
        .ok_or(Error::Protocol)?;
    serde_json::from_str(&text).map_err(|_| Error::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_invalid_ciphertext_and_padding() {
        for malformed in [b"0".as_slice(), b"not hex", b"0000000000000000"] {
            assert!(decode::<Reply>(malformed, b"FIXTURE!").is_err());
        }
    }
}
