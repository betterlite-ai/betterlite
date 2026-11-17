use std::fmt::{self, Formatter};

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{Deserializer, MapAccess, Visitor};
use thiserror::Error;

// bounded-inputs: the request metadata fragment is untrusted bytes, so it stops before the decoder
pub const MAX_METADATA_BYTES: usize = 1 << 20;

// LiteLLM's LAR1Metadata declares 0.5 for a request that names no confidence
const DEFAULT_CONFIDENCE: Decimal = Decimal::from_parts(5, 0, 0, false, 1);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Act {
    #[default]
    Inf,
    Obs,
    Ret,
    Gen,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Time {
    #[default]
    Now,
    Mem,
    Ctx,
    Pre,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Mind {
    #[default]
    Ref,
    Rec,
    Hyp,
    Act,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Evidence {
    Synth,
    Retrieved,
    Unverified,
    Confirmed,
}

// LAR1Metadata: every field has a declared default, and pydantic's model_validate ignores unknown keys
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Lar1Metadata {
    #[serde(default)]
    pub act: Act,
    #[serde(default)]
    pub time: Time,
    #[serde(default)]
    pub mind: Mind,
    #[serde(default = "default_confidence")]
    pub confidence: Decimal,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("the lar1 metadata is {length} bytes, over the {max} byte limit")]
    TooLarge { length: usize, max: usize },
    #[error("the lar1 metadata is not a valid LAR-1 object: {source}")]
    Invalid {
        #[source]
        source: serde_json::Error,
    },
    #[error("the lar1 confidence {confidence} is outside the range 0 to 1")]
    Confidence { confidence: Decimal },
}

const fn default_confidence() -> Decimal {
    DEFAULT_CONFIDENCE
}

// LiteLLM rejects a non-dict lar1 value, and serde would otherwise read a JSON array as a sequence
struct ObjectOnly(Lar1Metadata);

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for ObjectOnly {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ObjectVisitor;

        // ast-grep-ignore: no-custom-serde
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = ObjectOnly;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str("a LAR-1 metadata object")
            }

            fn visit_map<A>(self, map: A) -> Result<ObjectOnly, A::Error>
            where
                A: MapAccess<'de>,
            {
                Lar1Metadata::deserialize(MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }

        deserializer.deserialize_any(ObjectVisitor)
    }
}

impl Default for Lar1Metadata {
    fn default() -> Self {
        Self {
            act: Act::Inf,
            time: Time::Now,
            mind: Mind::Ref,
            confidence: DEFAULT_CONFIDENCE,
            evidence: Vec::new(),
        }
    }
}

impl Lar1Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Self, MetadataError> {
        let length = bytes.len();
        if length > MAX_METADATA_BYTES {
            return Err(MetadataError::TooLarge {
                length,
                max: MAX_METADATA_BYTES,
            });
        }
        let metadata: Self = serde_json::from_slice::<ObjectOnly>(bytes)
            .map_err(|source| MetadataError::Invalid { source })?
            .0;
        let confidence = metadata.confidence;
        if confidence < Decimal::ZERO || confidence > Decimal::ONE {
            return Err(MetadataError::Confidence { confidence });
        }
        Ok(metadata)
    }

    // _parse_lar1_metadata: malformed metadata warns and the request routes with the declared defaults
    #[must_use]
    pub fn from_request_metadata(bytes: &[u8]) -> Self {
        match Self::parse(bytes) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(%error, "invalid lar1 metadata, using the defaults");
                Self::default()
            }
        }
    }
}
