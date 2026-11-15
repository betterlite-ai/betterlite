use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::error::Category;

use crate::agent_365_tokens::error::{Agent365TokenError, Agent365TokenExchangeError};

pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(3599);
const DEFAULT_TOKEN_ERROR_CODE: &str = "invalid_grant";
pub const MAX_DESCRIPTION_CHARS: usize = 512;

#[derive(Debug, Deserialize)]
pub struct TokenReply {
    #[serde(default)]
    access_token: AccessTokenField,
    expires_in: Option<ExpiresInField>,
    error: Option<TextField>,
    error_description: Option<TextField>,
    error_codes: Option<CodesField>,
}

#[derive(Debug)]
pub struct TokenGrant {
    pub access_token: SecretString,
    pub expires_in: Duration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum AccessTokenField {
    Token(SecretString),
    Other(GuardrailValue),
    #[default]
    Missing,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExpiresInField {
    Number(f64),
    Text(String),
    Other(GuardrailValue),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TextField {
    Text(String),
    Other(GuardrailValue),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CodesField {
    Codes(Vec<CodeItem>),
    Other(GuardrailValue),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CodeItem {
    Integer(i64),
    Other(GuardrailValue),
}

pub fn decode_token_reply(body: &[u8]) -> Result<TokenReply, Agent365TokenError> {
    serde_json::from_slice(body).map_err(|source| {
        // serde reports a valid JSON body of the wrong shape as a data error and broken JSON as a syntax error
        if source.classify() == Category::Data {
            Agent365TokenError::NonObjectBody
        } else {
            Agent365TokenError::NonJsonBody { source }
        }
    })
}

impl TokenReply {
    pub fn into_grant(self) -> Result<TokenGrant, Agent365TokenError> {
        match self.access_token {
            AccessTokenField::Missing => Err(Agent365TokenError::MissingAccessToken),
            AccessTokenField::Token(token) if !token.expose_secret().is_empty() => Ok(TokenGrant {
                access_token: token,
                expires_in: parse_expires_in(self.expires_in),
            }),
            AccessTokenField::Token(_) | AccessTokenField::Other(_) => {
                Err(Agent365TokenError::BadAccessToken)
            }
        }
    }

    #[must_use]
    pub fn exchange_failure(&self, status: u16) -> Agent365TokenExchangeError {
        let description = self
            .error_description
            .as_ref()
            .and_then(TextField::text)
            .unwrap_or_default()
            .chars()
            .take(MAX_DESCRIPTION_CHARS)
            .collect();
        Agent365TokenExchangeError {
            status_code: status,
            error_code: self
                .error
                .as_ref()
                .and_then(TextField::text)
                .unwrap_or(DEFAULT_TOKEN_ERROR_CODE)
                .to_owned(),
            description,
            aadsts_codes: self.aadsts_codes(),
        }
    }

    fn aadsts_codes(&self) -> Vec<i64> {
        let Some(CodesField::Codes(items)) = &self.error_codes else {
            return Vec::new();
        };
        let parsed: Option<Vec<i64>> = items
            .iter()
            .map(|item| match item {
                CodeItem::Integer(code) => Some(*code),
                CodeItem::Other(_) => None,
            })
            .collect();
        parsed.unwrap_or_default()
    }
}

impl TextField {
    fn text(&self) -> Option<&str> {
        match self {
            TextField::Text(text) => Some(text),
            TextField::Other(_) => None,
        }
    }
}

fn parse_expires_in(raw: Option<ExpiresInField>) -> Duration {
    let seconds = match raw {
        Some(ExpiresInField::Number(seconds)) => seconds,
        Some(ExpiresInField::Text(text)) => match text.trim().parse::<f64>() {
            Ok(seconds) => seconds,
            Err(_parse) => return DEFAULT_TOKEN_TTL,
        },
        Some(ExpiresInField::Other(_)) | None => return DEFAULT_TOKEN_TTL,
    };
    Duration::try_from_secs_f64(seconds).unwrap_or(DEFAULT_TOKEN_TTL)
}
