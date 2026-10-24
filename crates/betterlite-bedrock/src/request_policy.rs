pub mod aws_params;
pub mod validation;

mod tests;

use std::{io, option};

use indexmap::{IndexMap, IndexSet};
use secrecy::SecretString;
use serde::Serializer as _;
use serde_json::ser::{Formatter, Serializer};

use crate::error::BedrockError;
use crate::request_policy::validation::{MAX_PAIRS, is_forwardable};

pub const REQUEST_METADATA_HEADER: &str = "X-Amzn-Bedrock-Request-Metadata";
const OWNED_HEADER: &str = "x-amzn-bedrock-request-metadata";
const IDENTITY_PREFIX: &str = "user_api_key_";
const SPEND_LOGS_FIELD: &str = "spend_logs_metadata";

pub type Params = IndexMap<String, RequestValue>;

#[derive(Clone, Debug)]
pub enum RequestValue {
    Text(String),
    Secret(SecretString),
    Mapping(Params),
    // any other JSON value as its source text, so a merge passes it through untouched
    Raw(String),
}

pub type OneResponse<T> = option::IntoIter<T>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BedrockRequestPolicy {
    fields: IndexSet<String>,
}

impl BedrockRequestPolicy {
    #[must_use]
    pub fn from_fields(configured: &[String]) -> Option<Self> {
        let fields: IndexSet<String> = configured.iter().cloned().collect();
        (!fields.is_empty()).then_some(Self { fields })
    }

    #[must_use]
    pub fn resolve(
        &self,
        litellm_params: &Params,
        caller: Option<&RequestValue>,
    ) -> Option<IndexMap<String, String>> {
        let sources: Vec<&Params> = ["metadata", "litellm_metadata"]
            .into_iter()
            .filter_map(|name| match litellm_params.get(name) {
                Some(RequestValue::Mapping(entries)) => Some(entries),
                Some(RequestValue::Text(_) | RequestValue::Secret(_) | RequestValue::Raw(_))
                | None => None,
            })
            .collect();
        let mut resolved = self.identity_pairs(&sources);
        let budget = MAX_PAIRS.saturating_sub(resolved.len());
        let spend_sources = if self.fields.contains(SPEND_LOGS_FIELD) {
            sources.as_slice()
        } else {
            &[]
        };
        let spend_pairs = spend_sources
            .iter()
            .flat_map(|entries| text_pairs(entries.get(SPEND_LOGS_FIELD)));
        let mut client: IndexMap<&String, &String> = IndexMap::new();
        for (key, value) in text_pairs(caller).into_iter().chain(spend_pairs) {
            if !client.contains_key(key)
                && client.len() < budget
                && !key.starts_with(IDENTITY_PREFIX)
                && is_forwardable(key, value)
            {
                client.insert(key, value);
            }
        }
        resolved.extend(
            client
                .into_iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        (!resolved.is_empty()).then_some(resolved)
    }

    fn identity_pairs(&self, sources: &[&Params]) -> IndexMap<String, String> {
        let mut identity: IndexMap<String, String> = IndexMap::new();
        for field in self
            .fields
            .iter()
            .filter(|field| field.starts_with(IDENTITY_PREFIX))
        {
            let first = sources.iter().find_map(|entries| match entries.get(field) {
                Some(RequestValue::Text(value)) => Some(value),
                Some(RequestValue::Secret(_) | RequestValue::Mapping(_) | RequestValue::Raw(_))
                | None => None,
            });
            if let Some(value) = first
                && identity.len() < MAX_PAIRS
                && is_forwardable(field, value)
            {
                identity.insert(field.clone(), value.clone());
            }
        }
        identity
    }

    pub fn metadata_header(&self, litellm_params: &Params) -> Result<Option<String>, BedrockError> {
        let Some(pairs) = self.resolve(litellm_params, None) else {
            return Ok(None);
        };
        let mut serializer = Serializer::with_formatter(Vec::new(), AsciiFormatter);
        serializer
            .collect_map(&pairs)
            .map_err(|source| BedrockError::MetadataEncoding { source })?;
        let header = String::from_utf8(serializer.into_inner())
            .map_err(|source| BedrockError::MetadataNotUtf8 { source })?;
        Ok(Some(header))
    }
}

pub fn merge_invoke_headers(
    headers: &IndexMap<String, String>,
    guardrail: &[(String, String)],
    policy: Option<&BedrockRequestPolicy>,
    litellm_params: &Params,
) -> Result<IndexMap<String, String>, BedrockError> {
    let mut merged: IndexMap<String, String> = headers
        .iter()
        .filter(|(name, _)| policy.is_none() || name.to_lowercase() != OWNED_HEADER)
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    for (name, value) in guardrail {
        let lowered = name.to_lowercase();
        if !headers
            .keys()
            .any(|existing| existing.to_lowercase() == lowered)
        {
            merged.insert(name.clone(), value.clone());
        }
    }
    if let Some(owner) = policy
        && let Some(value) = owner.metadata_header(litellm_params)?
    {
        merged.insert(REQUEST_METADATA_HEADER.to_owned(), value);
    }
    Ok(merged)
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

// Python's json.dumps escapes every character outside printable ASCII, and the header carries its exact bytes
struct AsciiFormatter;

impl Formatter for AsciiFormatter {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        for unit in fragment.encode_utf16() {
            match u8::try_from(unit).map(char::from) {
                Ok(character @ ' '..='~') => write!(writer, "{character}")?,
                Ok(_) | Err(_) => write!(writer, "\\u{unit:04x}")?,
            }
        }
        Ok(())
    }
}
