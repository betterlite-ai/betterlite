#[rustfmt::skip]
use crate::helpers::{HelperError, HelperResult};
#[rustfmt::skip]
use betterlite_mcp::errors::{SessionKeyError as KeyError, SessionMintError::TooLarge, SessionOpenError};
#[rustfmt::skip]
use betterlite_mcp::session_tokens::{mint_session_token as mint_token, open_session_token as open_token};
#[rustfmt::skip]
use betterlite_mcp::types::{AsymmetricSessionKeys, OpenedSessionToken, SessionKeys, SessionPrincipal, SessionRotatedPublicKey, SessionSigningKeys as SigningKeys, SessionTokenKind};
#[rustfmt::skip]
use chrono::{DateTime, Utc};
#[rustfmt::skip]
use secrecy::{ExposeSecret as _, SecretString};
#[rustfmt::skip]
use serde::{Deserialize, Serialize};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[rustfmt::skip]
pub(crate) struct Input { mint: Option<MintStep>, open: Option<OpenStep> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[rustfmt::skip]
struct MintStep {
    keys: Keys,
    kind: SessionTokenKind,
    principal: SessionPrincipal,
    #[serde(with = "chrono::serde::ts_seconds")]
    now: DateTime<Utc>,
    entropy: [u8; 16],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[rustfmt::skip]
struct OpenStep {
    keys: Keys,
    kind: SessionTokenKind,
    #[serde(with = "chrono::serde::ts_seconds")]
    now: DateTime<Utc>,
    candidate: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
#[rustfmt::skip]
enum Keys {
    Symmetric { signing_key: String },
    Asymmetric {
        private_key_pem: String,
        kid: String,
        #[serde(default)]
        previous_public_keys: Vec<RotatedKey>,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[rustfmt::skip]
struct RotatedKey { kid: String, public_key_pem: String }
#[derive(Serialize)]
#[serde(untagged)]
#[rustfmt::skip]
enum MintOutput {
    Minted { token: String, expires_at: i64, public_key_pem: Option<String> },
    TooLarge { tag: &'static str, size_bytes: usize, max_bytes: usize },
}
#[derive(Serialize)]
#[serde(untagged)]
#[rustfmt::skip]
enum OpenOutput { Opened(OpenedSessionToken), Error(SessionOpenError) }
#[derive(Serialize)]
#[rustfmt::skip]
struct Output { mint: Option<MintOutput>, open: Option<OpenOutput> }

#[rustfmt::skip]
pub(crate) struct Op;
#[rustfmt::skip]
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let minted = input.mint.as_ref().map(mint).transpose()?;
        let Some(step) = &input.open else {
            return serde_json::to_value(Output { mint: minted, open: None }).map_err(|error| {
                Box::new(HelperError { class: "Output".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() })
            });
        };
        let token = minted.as_ref().and_then(|output| match output {
            MintOutput::Minted { token, .. } => Some(token.as_str()),
            MintOutput::TooLarge { .. } => None,
        });
        let candidate = step.candidate.as_deref().or(token).unwrap_or_default();
        let keys = signing_keys(&step.keys)?;
        let open = match open_token(candidate, step.kind, &keys, step.now) {
            Ok(opened) => OpenOutput::Opened(opened),
            Err(error) => OpenOutput::Error(error),
        };
        serde_json::to_value(Output { mint: minted, open: Some(open) }).map_err(|error| {
            Box::new(HelperError { class: "Output".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() })
        })
    }
}

#[rustfmt::skip]
fn mint(step: &MintStep) -> Result<MintOutput, Box<HelperError>> {
    let keys = signing_keys(&step.keys)?;
    let public_key_pem = match &keys {
        SigningKeys::Symmetric(_) => Ok(None),
        SigningKeys::Asymmetric(asymmetric) => asymmetric.public_key_pem().map(Some),
    }.map_err(|error| Box::new(HelperError { class: "SessionKeyError".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))?;
    let entropy = step.entropy;
    match mint_token(&step.principal, step.kind, &keys, step.now, || Ok(entropy)) {
        Ok(minted) => Ok(MintOutput::Minted { token: minted.token.expose_secret().to_owned(), expires_at: minted.expires_at.timestamp(), public_key_pem }),
        Err(TooLarge { size_bytes, max_bytes }) => Ok(MintOutput::TooLarge { tag: "session_token_too_large", size_bytes, max_bytes }),
        Err(error) => Err(Box::new(HelperError { class: "SessionMintError".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() })),
    }
}

#[rustfmt::skip]
fn signing_keys(keys: &Keys) -> Result<SigningKeys, Box<HelperError>> {
    let built = match keys {
        Keys::Symmetric { signing_key } => SessionKeys::new(SecretString::from(signing_key.as_str())).map(SigningKeys::Symmetric),
        Keys::Asymmetric { private_key_pem, kid, previous_public_keys } => previous_public_keys.iter().map(|key| SessionRotatedPublicKey::new(key.kid.clone(), &key.public_key_pem)).collect::<Result<Vec<_>, KeyError>>().and_then(|previous| AsymmetricSessionKeys::new(&SecretString::from(private_key_pem.as_str()), kid.clone(), previous)).map(SigningKeys::Asymmetric),
    };
    built.map_err(|error| Box::new(HelperError { class: "ValidationError".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))
}
