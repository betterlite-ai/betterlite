use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::native_speech::identity::NativeSpeechIdentity;

#[must_use]
pub fn validate_environment(
    identity: &NativeSpeechIdentity,
    headers: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut validated = headers;
    if let Some(key) = identity
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty())
    {
        validated.insert(
            "Ocp-Apim-Subscription-Key".to_owned(),
            key.expose_secret().to_owned(),
        );
    }
    validated.insert("Content-Type".to_owned(), "application/ssml+xml".to_owned());
    validated.insert("User-Agent".to_owned(), "litellm".to_owned());
    validated
}
