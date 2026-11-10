use secrecy::{ExposeSecret as _, SecretString};

#[must_use]
pub fn resolve_bearer_token(
    api_key: Option<&SecretString>,
    mantle_api_key: Option<&SecretString>,
    aws_bearer_token: Option<&SecretString>,
) -> Option<SecretString> {
    let selected = [api_key, mantle_api_key, aws_bearer_token]
        .into_iter()
        .flatten()
        .find(|token| !token.expose_secret().is_empty());
    // Python's `a or b or c` evaluates to the last operand when none of them is truthy
    selected.or(aws_bearer_token).cloned()
}
