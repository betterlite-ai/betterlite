use betterlite_mcp::errors::{SessionKeyError, SessionMintError};
use betterlite_mcp::session_tokens::{JtiSource, mint_session_token, open_session_token};
use betterlite_mcp::types::{
    AsymmetricSessionKeys, SessionKeys, SessionPrincipal, SessionRotatedPublicKey,
    SessionSigningKeys, SessionTokenKind,
};
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult};

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
    now: i64,
    entropy: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenStep {
    keys: Keys,
    kind: SessionTokenKind,
    now: i64,
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

struct FixedJti([u8; 16]);

impl JtiSource for FixedJti {
    fn fill(&self, jti: &mut [u8; 16]) -> Result<(), getrandom::Error> {
        *jti = self.0;
        Ok(())
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let minted = input.mint.as_ref().map(mint).transpose()?;
        let token = minted
            .as_ref()
            .and_then(|value| value.get("token")?.as_str());
        let opened = input
            .open
            .as_ref()
            .map(|step| open(step, token))
            .transpose()?;
        Ok(serde_json::json!({"mint": minted, "open": opened}))
    }
}

fn open(step: &OpenStep, minted: Option<&str>) -> HelperResult {
    let candidate = step.candidate.as_deref().or(minted).unwrap_or_default();
    let now =
        DateTime::from_timestamp(step.now, 0).ok_or_else(|| failure("now", "out of range"))?;
    let keys = signing_keys(&step.keys)?;
    let encoded = match open_session_token(candidate, step.kind, &keys, now) {
        Ok(opened) => serde_json::to_value(opened),
        Err(error) => serde_json::to_value(error),
    };
    encoded.map_err(|error| failure("Output", &error.to_string()))
}

fn mint(step: &MintStep) -> HelperResult {
    let mut entropy = [0_u8; 16];
    hex::decode_to_slice(&step.entropy, &mut entropy)
        .map_err(|error| failure("entropy", &error.to_string()))?;
    let now =
        DateTime::from_timestamp(step.now, 0).ok_or_else(|| failure("now", "out of range"))?;
    let keys = signing_keys(&step.keys)?;
    let public_key_pem = match &keys {
        SessionSigningKeys::Symmetric(_) => None,
        SessionSigningKeys::Asymmetric(asymmetric) => Some(
            asymmetric
                .public_key_pem()
                .map_err(|error| failure("SessionKeyError", &error.to_string()))?,
        ),
    };
    match mint_session_token(&step.principal, step.kind, &keys, now, &FixedJti(entropy)) {
        Ok(minted) => Ok(serde_json::json!({
            "token": minted.token.expose_secret(),
            "expires_at": minted.expires_at.timestamp(),
            "public_key_pem": public_key_pem,
        })),
        Err(SessionMintError::TooLarge {
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

fn signing_keys(keys: &Keys) -> Result<SessionSigningKeys, Box<HelperError>> {
    let built = match keys {
        Keys::Symmetric { signing_key } => {
            let secret = SecretString::from(signing_key.as_str());
            SessionKeys::new(secret).map(SessionSigningKeys::Symmetric)
        }
        Keys::Asymmetric {
            private_key_pem,
            kid,
            previous_public_keys,
        } => previous_public_keys
            .iter()
            .map(|key| SessionRotatedPublicKey::new(key.kid.clone(), &key.public_key_pem))
            .collect::<Result<Vec<_>, SessionKeyError>>()
            .and_then(|previous| {
                let private = SecretString::from(private_key_pem.as_str());
                AsymmetricSessionKeys::new(&private, kid.clone(), previous)
            })
            .map(SessionSigningKeys::Asymmetric),
    };
    built.map_err(|error| failure("ValidationError", &error.to_string()))
}

fn failure(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
