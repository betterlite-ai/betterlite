use crate::helpers::{HelperError, HelperResult};
use betterlite_mcp::errors::SessionKeyError as KeyError;
use betterlite_mcp::errors::SessionMintError::TooLarge;
use betterlite_mcp::session_tokens::{
    mint_session_token as mint_token, open_session_token as open_token,
};
use betterlite_mcp::types::{
    AsymmetricSessionKeys, SessionKeys, SessionPrincipal, SessionRotatedPublicKey,
    SessionSigningKeys as SigningKeys, SessionTokenKind,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mint: Option<MintStep>,
    open: Option<OpenStep>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
struct OpenStep {
    keys: Keys,
    kind: SessionTokenKind,
    #[serde(with = "chrono::serde::ts_seconds")]
    now: DateTime<Utc>,
    candidate: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Keys {
    Symmetric {
        signing_key: String,
    },
    Asymmetric {
        private_key_pem: String,
        kid: String,
        #[serde(default)]
        previous_public_keys: Vec<RotatedKey>,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotatedKey {
    kid: String,
    public_key_pem: String,
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let minted = input.mint.as_ref().map(mint).transpose()?;
        let Some(step) = &input.open else {
            return Ok(serde_json::json!({"mint": minted, "open": null}));
        };
        let minted_token = minted.as_ref().and_then(|output| output.get("token")?.as_str());
        let candidate = step.candidate.as_deref().or(minted_token).unwrap_or_default();
        let keys = signing_keys(&step.keys)?;
        let opened = match open_token(candidate, step.kind, &keys, step.now) {
            Ok(opened) => serde_json::to_value(opened),
            Err(error) => serde_json::to_value(error),
        }
        .map_err(|error| failure("Output", &error.to_string()))?;
        Ok(serde_json::json!({"mint": minted, "open": opened}))
    }
}

fn mint(step: &MintStep) -> HelperResult {
    let keys = signing_keys(&step.keys)?;
    let public_key_pem = match &keys {
        SigningKeys::Symmetric(_) => Ok(None),
        SigningKeys::Asymmetric(asymmetric) => asymmetric.public_key_pem().map(Some),
    }
    .map_err(|error| failure("SessionKeyError", &error.to_string()))?;
    let entropy = step.entropy;
    match mint_token(&step.principal, step.kind, &keys, step.now, || Ok(entropy)) {
        Ok(minted) => Ok(serde_json::json!({
            "token": minted.token.expose_secret(),
            "expires_at": minted.expires_at.timestamp(),
            "public_key_pem": public_key_pem,
        })),
        Err(TooLarge {
            size_bytes,
            max_bytes,
        }) => Ok(serde_json::json!({
            "tag": "session_token_too_large",
            "size_bytes": size_bytes,
            "max_bytes": max_bytes,
        })),
        Err(error) => Err(failure("SessionMintError", &error.to_string())),
    }
}

fn signing_keys(keys: &Keys) -> Result<SigningKeys, Box<HelperError>> {
    let built = match keys {
        Keys::Symmetric { signing_key } => {
            SessionKeys::new(SecretString::from(signing_key.as_str())).map(SigningKeys::Symmetric)
        }
        Keys::Asymmetric {
            private_key_pem,
            kid,
            previous_public_keys,
        } => previous_public_keys
            .iter()
            .map(|key| SessionRotatedPublicKey::new(key.kid.clone(), &key.public_key_pem))
            .collect::<Result<Vec<_>, KeyError>>()
            .and_then(|previous| {
                let private = SecretString::from(private_key_pem.as_str());
                AsymmetricSessionKeys::new(&private, kid.clone(), previous)
            })
            .map(SigningKeys::Asymmetric),
    };
    built.map_err(|error| failure("ValidationError", &error.to_string()))
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    let error = HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
