use bytes::BytesMut;
use secrecy::{ExposeSecret, SecretString};
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use subtle::ConstantTimeEq as _;
use tokio_postgres::types::{IsNull, ToSql, Type, to_sql_checked};

// The stored identifier is already hashed or is a legacy literal key, so nothing here rehashes it.
#[derive(Clone)]
pub struct KeyHash(SecretString);

impl From<&str> for KeyHash {
    fn from(identifier: &str) -> Self {
        Self(SecretString::from(identifier))
    }
}

impl From<String> for KeyHash {
    fn from(identifier: String) -> Self {
        Self(SecretString::from(identifier))
    }
}

impl ExposeSecret<str> for KeyHash {
    fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

impl PartialEq for KeyHash {
    fn eq(&self, other: &Self) -> bool {
        bool::from(
            self.0
                .expose_secret()
                .as_bytes()
                .ct_eq(other.0.expose_secret().as_bytes()),
        )
    }
}

impl Eq for KeyHash {}

impl Hash for KeyHash {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.0.expose_secret().as_bytes().hash(state);
    }
}

impl fmt::Debug for KeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl ToSql for KeyHash {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Send + Sync>> {
        self.0.expose_secret().to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    to_sql_checked!();
}
