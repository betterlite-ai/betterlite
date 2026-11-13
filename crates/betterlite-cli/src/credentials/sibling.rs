use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use super::contracts::CliTokenData;

#[must_use]
pub fn key_rotated_by_a_sibling(
    reloaded: Option<&CliTokenData>,
    previous: &CliTokenData,
    now_seconds: f64,
) -> Option<SecretString> {
    let candidate = reloaded?;
    if same_secret(
        candidate.refresh_token.as_ref(),
        previous.refresh_token.as_ref(),
    ) {
        return None;
    }
    if candidate.base_url != previous.base_url
        || candidate.token_endpoint != previous.token_endpoint
        || candidate.resource != previous.resource
        || candidate.user_id != previous.user_id
        || candidate.team_id != previous.team_id
    {
        return None;
    }
    if now_seconds >= candidate.expires_at? {
        return None;
    }
    let key = candidate.key.as_ref()?;
    if key.expose_secret().is_empty() {
        return None;
    }
    Some(key.clone())
}

fn same_secret(left: Option<&SecretString>, right: Option<&SecretString>) -> bool {
    match (left, right) {
        (Some(left_token), Some(right_token)) => bool::from(
            left_token
                .expose_secret()
                .as_bytes()
                .ct_eq(right_token.expose_secret().as_bytes()),
        ),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
