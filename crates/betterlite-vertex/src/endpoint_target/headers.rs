use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

#[must_use]
pub fn set_headers(
    auth_header: Option<&SecretString>,
    extra_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, SecretString> {
    let mut headers = BTreeMap::new();
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    if let Some(token) = auth_header {
        headers.insert(
            "Authorization".to_owned(),
            SecretString::from(format!("Bearer {}", token.expose_secret())),
        );
    }
    for (name, value) in extra_headers {
        headers.insert(name.clone(), SecretString::from(value.clone()));
    }
    headers
}
