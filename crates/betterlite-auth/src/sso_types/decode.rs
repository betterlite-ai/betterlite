use serde::de::DeserializeOwned;
use serde_json::Deserializer;
use thiserror::Error;

use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::SsoConfig;

pub const MAX_SSO_RECORD_BYTES: usize = 0x0010_0000;

#[derive(Debug, Error)]
pub enum SsoRecordError {
    #[error("the SSO record is {bytes} bytes; the maximum is {limit}")]
    TooLarge { bytes: usize, limit: usize },
    #[error("the SSO record kind tag {tag} is unknown")]
    UnknownKind { tag: u8 },
    #[error("the SSO record has bytes after its JSON value: {0}")]
    Trailing(#[source] serde_json::Error),
    #[error("the SSO record does not match its shape: {0}")]
    Record(#[from] serde_path_to_error::Error<serde_json::Error>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoRecordKind {
    Settings,
    Identity,
}

impl TryFrom<u8> for SsoRecordKind {
    type Error = SsoRecordError;

    fn try_from(tag: u8) -> Result<Self, Self::Error> {
        match tag {
            0 => Ok(Self::Settings),
            1 => Ok(Self::Identity),
            other => Err(SsoRecordError::UnknownKind { tag: other }),
        }
    }
}

#[derive(Debug)]
pub enum SsoRecord {
    Settings(Box<SsoConfig>),
    Identity(Box<OpenIdIdentity>),
}

impl SsoRecordKind {
    pub fn decode(self, bytes: &[u8]) -> Result<SsoRecord, SsoRecordError> {
        match self {
            Self::Settings => {
                decode_sso_record(bytes).map(|config| SsoRecord::Settings(Box::new(config)))
            }
            Self::Identity => decode_open_id_identity(bytes)
                .map(|identity| SsoRecord::Identity(Box::new(identity))),
        }
    }
}

pub fn decode_sso_record(bytes: &[u8]) -> Result<SsoConfig, SsoRecordError> {
    check_size(bytes)?;
    decode_record(bytes)
}

pub fn decode_open_id_identity(bytes: &[u8]) -> Result<OpenIdIdentity, SsoRecordError> {
    check_size(bytes)?;
    decode_record(bytes)
}

const fn check_size(bytes: &[u8]) -> Result<(), SsoRecordError> {
    if bytes.len() > MAX_SSO_RECORD_BYTES {
        return Err(SsoRecordError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_SSO_RECORD_BYTES,
        });
    }
    Ok(())
}

fn decode_record<T>(bytes: &[u8]) -> Result<T, SsoRecordError>
where
    T: DeserializeOwned,
{
    let mut decoder = Deserializer::from_slice(bytes);
    let record: T = serde_path_to_error::deserialize(&mut decoder)?;
    decoder.end().map_err(SsoRecordError::Trailing)?;
    Ok(record)
}
