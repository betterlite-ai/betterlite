use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone, Debug)]
pub struct Header {
    pub name: String,
    pub value: SecretString,
}

#[must_use]
pub fn plain_headers(pairs: &[(&str, &str)]) -> Vec<Header> {
    pairs
        .iter()
        .map(|(name, value)| Header {
            name: (*name).to_owned(),
            value: SecretString::from(*value),
        })
        .collect()
}

#[must_use]
pub fn json_headers(credential_name: &str, credential: SecretString) -> Vec<Header> {
    let mut headers = plain_headers(&[
        ("accept", "application/json"),
        ("Content-Type", "application/json"),
    ]);
    headers.push(Header {
        name: credential_name.to_owned(),
        value: credential,
    });
    headers
}

// A value of 12 characters or fewer would be shown in full by the 8 and 4 split, so it is masked whole.
#[must_use]
pub fn redact_headers(headers: &[Header]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|header| {
            let value = header.value.expose_secret();
            let name = header.name.to_lowercase();
            if !matches!(name.as_str(), "authorization" | "galileo-api-key") || value.is_empty() {
                return (header.name.clone(), value.to_owned());
            }
            let characters: Vec<char> = value.chars().collect();
            if characters.len() <= 12 {
                return (header.name.clone(), "***".to_owned());
            }
            let head: String = characters.iter().take(8).collect();
            let tail: String = characters
                .iter()
                .skip(characters.len().saturating_sub(4))
                .collect();
            (header.name.clone(), format!("{head}...{tail}"))
        })
        .collect()
}
