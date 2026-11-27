//! The framing every versioned stored form shares: its format number, then
//! its body as a value of its own, so a reader checks the number before it
//! decodes a body whose layout it may not know.

use musli::Encode;
use musli::storage::Error;

/// `body` written after `format`.
pub(crate) fn to_bytes<T>(format: u32, body: &T) -> Result<Vec<u8>, Error>
where
    T: Encode<musli::mode::Binary>,
{
    let mut bytes = musli::storage::to_vec(&format)?;
    bytes.extend(musli::storage::to_vec(body)?);
    Ok(bytes)
}

/// The format number `bytes` start with, and the body after it.
pub(crate) fn split(bytes: &[u8]) -> Result<(u32, &[u8]), Error> {
    let mut reader = musli::reader::SliceReader::new(bytes);
    let format = musli::storage::decode(&mut reader)?;
    Ok((format, reader.as_slice()))
}

/// The body of `bytes`, whatever their format number.
#[cfg(test)]
pub(crate) fn body<'de, T>(bytes: &'de [u8]) -> T
where
    T: musli::Decode<'de, musli::mode::Binary, musli::alloc::Global>,
{
    let (_, body) = split(bytes).unwrap();
    musli::storage::from_slice(body).unwrap()
}
