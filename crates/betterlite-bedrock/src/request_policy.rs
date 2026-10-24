pub mod aws_params;
pub mod validation;

mod tests;

use std::option;

use secrecy::SecretString;

use crate::request_policy::validation::{MAX_PAIRS, is_forwardable};

pub const REQUEST_METADATA_HEADER: &str = "X-Amzn-Bedrock-Request-Metadata";
const OWNED_HEADER: &str = "x-amzn-bedrock-request-metadata";
const IDENTITY_PREFIX: &str = "user_api_key_";
const SPEND_LOGS_FIELD: &str = "spend_logs_metadata";

#[derive(Clone, Debug)]
pub enum RequestValue {
    Text(String),
    Secret(SecretString),
    Mapping(Vec<(String, Self)>),
    // any other JSON value as its source text, so a merge passes it through untouched
    Raw(String),
}

pub type OneResponse<T> = option::IntoIter<T>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BedrockRequestPolicy {
    fields: Vec<String>,
}

impl BedrockRequestPolicy {
    #[must_use]
    pub fn from_fields(configured: &[String]) -> Option<Self> {
        let mut fields: Vec<String> = Vec::new();
        for field in configured {
            if !fields.contains(field) {
                fields.push(field.clone());
            }
        }
        (!fields.is_empty()).then_some(Self { fields })
    }

    #[must_use]
    pub fn resolve(
        &self,
        litellm_params: &[(String, RequestValue)],
        caller: Option<&RequestValue>,
    ) -> Option<Vec<(String, String)>> {
        let sources: Vec<&[(String, RequestValue)]> = ["metadata", "litellm_metadata"]
            .into_iter()
            .filter_map(|name| match lookup(litellm_params, name) {
                Some(RequestValue::Mapping(entries)) => Some(entries.as_slice()),
                Some(RequestValue::Text(_) | RequestValue::Secret(_) | RequestValue::Raw(_))
                | None => None,
            })
            .collect();
        let mut resolved = self.identity_pairs(&sources);
        let budget = MAX_PAIRS.saturating_sub(resolved.len());
        let spend_sources = if self.fields.iter().any(|field| field == SPEND_LOGS_FIELD) {
            sources.as_slice()
        } else {
            &[]
        };
        let spend_pairs = spend_sources
            .iter()
            .flat_map(|entries| text_pairs(lookup(entries, SPEND_LOGS_FIELD)));
        let mut client: Vec<(String, String)> = Vec::new();
        for (key, value) in text_pairs(caller).into_iter().chain(spend_pairs) {
            let fresh = !client.iter().any(|(earlier, _)| earlier == key);
            if fresh
                && client.len() < budget
                && !key.starts_with(IDENTITY_PREFIX)
                && is_forwardable(key, value)
            {
                client.push((key.clone(), value.clone()));
            }
        }
        resolved.extend(client);
        (!resolved.is_empty()).then_some(resolved)
    }

    fn identity_pairs(&self, sources: &[&[(String, RequestValue)]]) -> Vec<(String, String)> {
        let mut identity: Vec<(String, String)> = Vec::new();
        for field in self
            .fields
            .iter()
            .filter(|field| field.starts_with(IDENTITY_PREFIX))
        {
            let first = sources
                .iter()
                .find_map(|entries| match lookup(entries, field) {
                    Some(RequestValue::Text(value)) => Some(value),
                    Some(
                        RequestValue::Secret(_) | RequestValue::Mapping(_) | RequestValue::Raw(_),
                    )
                    | None => None,
                });
            if let Some(value) = first
                && identity.len() < MAX_PAIRS
                && is_forwardable(field, value)
            {
                identity.push((field.clone(), value.clone()));
            }
        }
        identity
    }

    #[must_use]
    pub fn metadata_header(&self, litellm_params: &[(String, RequestValue)]) -> Option<String> {
        let pairs = self.resolve(litellm_params, None)?;
        let body: Vec<String> = pairs
            .iter()
            .map(|(key, value)| format!("\"{}\":\"{}\"", json_text(key), json_text(value)))
            .collect();
        Some(format!("{{{}}}", body.join(",")))
    }
}

#[must_use]
pub fn merge_invoke_headers(
    headers: &[(String, String)],
    guardrail: &[(String, String)],
    policy: Option<&BedrockRequestPolicy>,
    litellm_params: &[(String, RequestValue)],
) -> Vec<(String, String)> {
    let existing: Vec<String> = headers
        .iter()
        .map(|(name, _)| name.to_lowercase())
        .collect();
    let mut merged: Vec<(String, String)> = Vec::new();
    for (name, value) in headers {
        if policy.is_none() || name.to_lowercase() != OWNED_HEADER {
            upsert(&mut merged, name, value.clone());
        }
    }
    for (name, value) in guardrail {
        if !existing.contains(&name.to_lowercase()) {
            upsert(&mut merged, name, value.clone());
        }
    }
    if let Some(value) = policy.and_then(|owner| owner.metadata_header(litellm_params)) {
        upsert(&mut merged, REQUEST_METADATA_HEADER, value);
    }
    merged
}

pub(crate) fn lookup<'params>(
    params: &'params [(String, RequestValue)],
    name: &str,
) -> Option<&'params RequestValue> {
    let (_, value) = params.iter().find(|(key, _)| key == name)?;
    Some(value)
}

pub(crate) fn upsert<V>(entries: &mut Vec<(String, V)>, name: &str, value: V) {
    match entries.iter_mut().find(|(key, _)| key == name) {
        Some((_, slot)) => *slot = value,
        None => entries.push((name.to_owned(), value)),
    }
}

fn text_pairs(value: Option<&RequestValue>) -> Vec<(&String, &String)> {
    let Some(RequestValue::Mapping(entries)) = value else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(key, entry)| match entry {
            RequestValue::Text(text) => Some((key, text)),
            RequestValue::Secret(_) | RequestValue::Mapping(_) | RequestValue::Raw(_) => None,
        })
        .collect()
}

// Python's json.dumps escapes every non-ASCII character, and the header carries its exact bytes
fn json_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\u{c}' => escaped.push_str("\\f"),
            ' '..='~' => escaped.push(character),
            other => {
                let mut units = [0; 2];
                let encoded = other.encode_utf16(&mut units).iter();
                escaped.extend(encoded.map(|unit| format!("\\u{unit:04x}")));
            }
        }
    }
    escaped
}
