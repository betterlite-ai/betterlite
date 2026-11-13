use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

// A bool on the wire; the newtype keeps struct_excessive_bools off a row that declares several
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowFlag(pub bool);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicInternet(pub bool);

impl Default for PublicInternet {
    fn default() -> Self {
        Self(true)
    }
}

pub type SecretMap = BTreeMap<String, SecretString>;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SecretMapField(
    #[serde(
        serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecretMap>>::serialize"
    )]
    pub SecretMap,
);

impl Debug for SecretMapField {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("SecretMapField([REDACTED])")
    }
}

// The plaintext view exists only during explicit storage serialization and its Debug prints nothing
#[derive(Serialize)]
#[serde(transparent)]
struct ExposedSecretMap(BTreeMap<String, String>);

impl From<&SecretMap> for ExposedSecretMap {
    fn from(map: &SecretMap) -> Self {
        Self(
            map.iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect(),
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct WireText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowTimestamp(
    #[serde(with = "serde_with::As::<serde_with::TryFromIntoRef<WireText>>")] TimestampPoint,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimestampPoint {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

impl TryFrom<WireText> for TimestampPoint {
    type Error = chrono::ParseError;

    fn try_from(text: WireText) -> Result<Self, Self::Error> {
        if let Ok(aware) = DateTime::parse_from_rfc3339(&text.0) {
            return Ok(Self::Aware(aware));
        }
        NaiveDateTime::parse_from_str(&text.0, "%Y-%m-%dT%H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(&text.0, "%Y-%m-%d %H:%M:%S%.f"))
            .map(Self::Naive)
    }
}

impl From<&TimestampPoint> for WireText {
    fn from(value: &TimestampPoint) -> Self {
        Self(match value {
            TimestampPoint::Aware(aware) => format!(
                "{}{}",
                format_naive(aware.naive_local()),
                format_offset(*aware)
            ),
            TimestampPoint::Naive(naive) => format_naive(*naive),
        })
    }
}

// pydantic writes six fractional digits and Z for a zero offset, unlike chrono's default serializer
fn format_naive(value: NaiveDateTime) -> String {
    if value.and_utc().timestamp_subsec_nanos() == 0 {
        return value.format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    value.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

fn format_offset(value: DateTime<FixedOffset>) -> String {
    if value.offset().local_minus_utc() == 0_i32 {
        return "Z".to_owned();
    }
    value.format("%:z").to_string()
}
