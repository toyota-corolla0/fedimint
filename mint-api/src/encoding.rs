//! This module defines a binary encoding interface which is more suitable for consensus critical
//! encoding thant e.g. `bincode`. Over time all structs that need to be encoded to binary will
//! be migrated to this interface.

use std::fmt::Formatter;
use std::io::Error;
use thiserror::Error;

/// Data which can be encoded in a consensus-consistent way
pub trait Encodable {
    /// Encode an object with a well-defined format.
    /// Returns the number of bytes written on success.
    ///
    /// The only errors returned are errors propagated from the writer.
    fn consensus_encode<W: std::io::Write>(&self, writer: W) -> Result<usize, std::io::Error>;
}

/// Data which can be encoded in a consensus-consistent way
pub trait Decodable: Sized {
    /// Decode an object with a well-defined format
    fn consensus_decode<D: std::io::Read>(d: D) -> Result<Self, DecodeError>;
}

#[derive(Debug, Error)]
pub struct DecodeError(Box<dyn std::error::Error>);

macro_rules! impl_encode_decode_bridge {
    ($btc_type:ty) => {
        impl crate::encoding::Encodable for $btc_type {
            fn consensus_encode<W: std::io::Write>(&self, writer: W) -> Result<usize, Error> {
                bitcoin::consensus::Encodable::consensus_encode(self, writer)
            }
        }

        impl crate::encoding::Decodable for $btc_type {
            fn consensus_decode<D: std::io::Read>(
                d: D,
            ) -> Result<Self, crate::encoding::DecodeError> {
                bitcoin::consensus::Decodable::consensus_decode(d).map_err(DecodeError::from_err)
            }
        }
    };
}

impl_encode_decode_bridge!(bitcoin::BlockHeader);
impl_encode_decode_bridge!(bitcoin::Script);
impl_encode_decode_bridge!(bitcoin::Transaction);
impl_encode_decode_bridge!(bitcoin::util::merkleblock::PartialMerkleTree);

macro_rules! impl_encode_num {
    ($num_type:ty) => {
        impl Encodable for $num_type {
            fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
                let bytes = self.to_le_bytes();
                writer.write_all(&bytes[..])?;
                Ok(bytes.len())
            }
        }
    };
}

impl_encode_num!(u64);

impl<T> Encodable for &[T]
where
    T: Encodable,
{
    fn consensus_encode<W: std::io::Write>(&self, mut writer: W) -> Result<usize, Error> {
        let mut len = 0;
        len += (self.len() as u64).consensus_encode(&mut writer)?;
        for item in self.iter() {
            len += item.consensus_encode(&mut writer)?;
        }
        Ok(len)
    }
}

impl Encodable for bitcoin::Amount {
    fn consensus_encode<W: std::io::Write>(&self, writer: W) -> Result<usize, Error> {
        self.as_sat().consensus_encode(writer)
    }
}

impl DecodeError {
    pub fn from_str(s: &'static str) -> Self {
        #[derive(Debug)]
        struct StrError(&'static str);

        impl std::fmt::Display for StrError {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl std::error::Error for StrError {}

        DecodeError(Box::new(StrError(s)))
    }

    pub fn from_err<E: std::error::Error + Send + 'static>(e: E) -> Self {
        DecodeError(Box::new(e))
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}
