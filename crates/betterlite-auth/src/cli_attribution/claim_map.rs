use betterlite_db::budget_permission_rows::JsonData;

const MAX_SCALAR_LENGTH: usize = 1024;
const SECRET_FRAGMENTS: [&str; 8] = [
    "access_token",
    "api_key",
    "client_secret",
    "id_token",
    "password",
    "private_key",
    "refresh_token",
    "secret",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliClaim {
    pub source: String,
    pub destination: String,
}

// One comma-separated source->destination entry per claim, destinations optionally prefixed with metadata.
#[must_use]
pub fn decode_cli_claim_map(text: &str) -> Vec<CliClaim> {
    let mut claims = Vec::new();
    for entry in text.trim().split(',') {
        if let Some(claim) = parse_claim(entry) {
            claims.push(claim);
        }
    }
    claims
}

fn parse_claim(entry: &str) -> Option<CliClaim> {
    let (source_field, destination_field) = entry.trim().split_once("->")?;
    let source = source_field.trim();
    let destination = destination_field.trim();
    let unprefixed = destination.strip_prefix("metadata.").unwrap_or(destination);
    if source.is_empty() || unprefixed.is_empty() {
        return None;
    }
    Some(CliClaim {
        source: source.to_owned(),
        destination: unprefixed.to_owned(),
    })
}

// The pinned destination shape is [A-Za-z0-9_.-]+, and no lowercased key may carry a secret fragment.
#[must_use]
pub fn is_safe_destination(destination: &str) -> bool {
    if destination.is_empty()
        || !destination.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
    {
        return false;
    }
    let lowered = destination.to_lowercase();
    !SECRET_FRAGMENTS
        .into_iter()
        .any(|fragment| lowered.contains(fragment))
}

// Python counts characters, so a string of 1024 multi-byte characters still fits.
#[must_use]
pub fn is_safe_claim_value(value: &JsonData) -> bool {
    match value {
        JsonData::Bool(_) | JsonData::Number(_) => true,
        JsonData::String(text) => {
            text.chars().count() <= MAX_SCALAR_LENGTH
                && !(text.starts_with("eyJ") && text.matches('.').count() >= 2)
        }
        JsonData::Null | JsonData::Array(_) | JsonData::Object(_) => false,
    }
}
