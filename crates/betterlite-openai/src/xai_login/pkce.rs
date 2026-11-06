use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::TryRng as _;
use rand::rngs::SysRng;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use url::form_urlencoded;

use super::error::XaiLoginError;
use crate::xai_tokens::rules::CLIENT_ID;

const ENTROPY_BYTES: usize = 32;

pub trait Entropy: Send + Sync {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), XaiLoginError>;
}

pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), XaiLoginError> {
        SysRng
            .try_fill_bytes(bytes)
            .map_err(|source| XaiLoginError::Entropy { source })
    }
}

pub fn pkce_pair<E>(entropy: &E) -> Result<(SecretString, String), XaiLoginError>
where
    E: Entropy,
{
    let mut random = [0_u8; ENTROPY_BYTES];
    entropy.fill(&mut random)?;
    Ok(pkce_pair_from(&random))
}

#[must_use]
pub fn pkce_pair_from(random: &[u8]) -> (SecretString, String) {
    let verifier = URL_SAFE_NO_PAD.encode(random);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (SecretString::from(verifier), challenge)
}

#[must_use]
pub fn build_authorize_url(
    authorization_endpoint: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    nonce: &str,
) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("response_type", "code");
    query.append_pair("client_id", CLIENT_ID);
    query.append_pair("redirect_uri", redirect_uri);
    query.append_pair(
        "scope",
        "openid profile email offline_access grok-cli:access api:access",
    );
    query.append_pair("code_challenge", challenge);
    query.append_pair("code_challenge_method", "S256");
    query.append_pair("state", state);
    query.append_pair("nonce", nonce);
    format!("{authorization_endpoint}?{}", query.finish())
}
