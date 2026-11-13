use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;

#[derive(Debug, Clone)]
pub struct ImdsToken {
    pub access_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
}

#[derive(Debug, Clone)]
pub enum ImdsReply {
    Token(ImdsToken),
    Error { error: String, description: String },
    Missing,
}

#[derive(Debug, thiserror::Error)]
pub enum ImdsTokenError {
    #[error("the IMDS token response is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the IMDS token response is not an object")]
    NotAnObject,
    #[error("the IMDS token response field {field} has the wrong type")]
    FieldType { field: &'static str },
    #[error("the IMDS token response field {field} is not an integer")]
    Integer { field: &'static str },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum Scalar {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
    Nested(IgnoredAny),
}

impl Scalar {
    fn text(self, field: &'static str) -> Result<String, ImdsTokenError> {
        match self {
            Self::Text(text) => Ok(text),
            Self::Number(_) | Self::Bool(_) | Self::Nested(_) => {
                Err(ImdsTokenError::FieldType { field })
            }
        }
    }

    fn integer(self, field: &'static str) -> Result<i64, ImdsTokenError> {
        if let Self::Number(number) = self {
            return number.as_i64().ok_or(ImdsTokenError::Integer { field });
        }
        // MSAL converts a string field with int(), which trims surrounding whitespace
        self.text(field)?
            .trim()
            .parse::<i64>()
            .map_err(|_error| ImdsTokenError::Integer { field })
    }
}

#[derive(Debug, Deserialize)]
struct RawReply {
    #[serde(default)]
    access_token: Option<Scalar>,
    #[serde(default)]
    expires_in: Option<Scalar>,
    #[serde(default)]
    token_type: Option<Scalar>,
    #[serde(default)]
    error: Option<Scalar>,
    #[serde(default)]
    error_description: Option<Scalar>,
    #[serde(flatten)]
    _extra: IndexMap<String, IgnoredAny>,
}

impl RawReply {
    fn reply(self) -> Result<ImdsReply, ImdsTokenError> {
        if let (Some(access_token), Some(expires_in)) = (self.access_token, self.expires_in) {
            let token_type = match self.token_type {
                Some(kind) => kind.text("token_type")?,
                None => "Bearer".to_owned(),
            };
            return Ok(ImdsReply::Token(ImdsToken {
                access_token: SecretString::from(access_token.text("access_token")?),
                expires_in: expires_in.integer("expires_in")?,
                token_type,
            }));
        }
        let Some(error) = self.error else {
            return Ok(ImdsReply::Missing);
        };
        let description = match self.error_description {
            Some(description) => description.text("error_description")?,
            None => String::new(),
        };
        Ok(ImdsReply::Error {
            error: error.text("error")?,
            description,
        })
    }
}

fn reply_error(error: serde_path_to_error::Error<serde_json::Error>) -> ImdsTokenError {
    if error.inner().classify() == Category::Data {
        ImdsTokenError::NotAnObject
    } else {
        ImdsTokenError::Json(error.into_inner())
    }
}

pub fn decode_imds_token(body: &[u8]) -> Result<ImdsReply, ImdsTokenError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let raw = match serde_path_to_error::deserialize::<_, RawReply>(&mut deserializer) {
        Ok(raw) => raw,
        Err(error) => return Err(reply_error(error)),
    };
    deserializer.end().map_err(ImdsTokenError::Json)?;
    raw.reply()
}
