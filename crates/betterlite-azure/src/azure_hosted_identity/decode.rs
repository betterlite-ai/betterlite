use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;

use super::error::HostedIdentityError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Expiry {
    Whole(i64),
    Text(String),
}

impl Expiry {
    pub fn seconds(&self, field: &'static str) -> Result<i64, HostedIdentityError> {
        match self {
            Self::Whole(seconds) => Ok(*seconds),
            Self::Text(text) => {
                text.trim()
                    .parse::<i64>()
                    .map_err(|_error| HostedIdentityError::ReplyValue {
                        field: field.to_owned(),
                    })
            }
        }
    }

    pub fn nonzero_seconds(
        slot: Option<&Self>,
        field: &'static str,
    ) -> Result<Option<i64>, HostedIdentityError> {
        let live = slot.filter(|expiry| match expiry {
            Self::Whole(seconds) => *seconds != 0,
            Self::Text(text) => !text.is_empty(),
        });
        live.map(|expiry| expiry.seconds(field)).transpose()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ReplyScalar {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceFabricError {
    #[serde(default)]
    pub(crate) code: Option<ReplyScalar>,
    #[serde(flatten)]
    pub(crate) _other: IndexMap<String, IgnoredAny>,
}

#[derive(Clone, Deserialize)]
pub struct HostedTokenReply {
    #[serde(default)]
    pub(crate) access_token: Option<String>,
    #[serde(default)]
    pub(crate) expires_on: Option<Expiry>,
    #[serde(default)]
    pub(crate) expires_in: Option<Expiry>,
    #[serde(default)]
    pub(crate) refresh_in: Option<Expiry>,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    #[serde(default)]
    pub(crate) resource: Option<String>,
    #[serde(default, rename = "statusCode")]
    pub(crate) status_code: Option<ReplyScalar>,
    #[serde(default)]
    pub(crate) message: Option<ReplyScalar>,
    #[serde(default)]
    pub(crate) error: Option<ServiceFabricError>,
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, IgnoredAny>,
}

impl HostedTokenReply {
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.access_token
            .as_deref()
            .filter(|candidate| !candidate.is_empty())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.access_token.is_none()
            && self.expires_on.is_none()
            && self.expires_in.is_none()
            && self.refresh_in.is_none()
            && self.token_type.is_none()
            && self.resource.is_none()
            && self.status_code.is_none()
            && self.message.is_none()
            && self.error.is_none()
            && self.extra.is_empty()
    }
}

fn reply_error(error: serde_path_to_error::Error<serde_json::Error>) -> HostedIdentityError {
    let field = error.path().to_string();
    if error.inner().classify() == Category::Data {
        HostedIdentityError::ReplyValue { field }
    } else {
        HostedIdentityError::ReplyJson(error.into_inner())
    }
}

pub fn decode_hosted_identity_token(
    body: &[u8],
) -> Result<Option<HostedTokenReply>, HostedIdentityError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let reply =
        match serde_path_to_error::deserialize::<_, Option<HostedTokenReply>>(&mut deserializer) {
            Ok(reply) => reply,
            Err(error) => return Err(reply_error(error)),
        };
    deserializer.end().map_err(HostedIdentityError::ReplyJson)?;
    Ok(reply)
}
