use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::Headers;
use super::error::BetaFilterError;
use crate::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaDropReason {
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DroppedBeta {
    pub token: String,
    pub reason: BetaDropReason,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilteredBetas {
    pub tokens: Vec<String>,
    pub dropped: Vec<DroppedBeta>,
}

pub fn filter_beta_tokens(
    matrix: &BetaMatrix,
    provider: &str,
    beta_headers: &[String],
) -> Result<FilteredBetas, BetaFilterError> {
    if beta_headers.is_empty() {
        return Ok(FilteredBetas::default());
    }
    let resolved = matrix.provider_name(provider);
    let mapping = provider_entries(matrix, resolved)?;
    let mut filtered = FilteredBetas::default();
    let mut tokens = BTreeSet::new();
    for header in beta_headers {
        map_beta_token(
            mapping,
            resolved,
            header.trim(),
            &mut tokens,
            &mut filtered.dropped,
        )?;
    }
    filtered.tokens = tokens.into_iter().collect();
    Ok(filtered)
}

fn provider_entries<'matrix>(
    matrix: &'matrix BetaMatrix,
    provider: &str,
) -> Result<Option<&'matrix IndexMap<String, BetaNode>>, BetaFilterError> {
    match matrix.0.get(provider) {
        None => Ok(None),
        Some(BetaNode::Map(entries)) => Ok(Some(entries)),
        Some(
            BetaNode::Null
            | BetaNode::Flag(_)
            | BetaNode::Number(_)
            | BetaNode::Text(_)
            | BetaNode::List(_),
        ) => Err(BetaFilterError::ProviderMapping {
            provider: provider.to_owned(),
        }),
    }
}

fn map_beta_token(
    mapping: Option<&IndexMap<String, BetaNode>>,
    provider: &str,
    token: &str,
    tokens: &mut BTreeSet<String>,
    dropped: &mut Vec<DroppedBeta>,
) -> Result<(), BetaFilterError> {
    match mapping.and_then(|entries| entries.get(token)) {
        None => {
            tracing::debug!(token, provider, "Dropping unknown beta header for provider");
            dropped.push(DroppedBeta {
                token: token.to_owned(),
                reason: BetaDropReason::Unknown,
            });
        }
        Some(BetaNode::Null) => {
            tracing::debug!(
                token,
                provider,
                "Dropping unsupported beta header for provider"
            );
            dropped.push(DroppedBeta {
                token: token.to_owned(),
                reason: BetaDropReason::Unsupported,
            });
        }
        Some(BetaNode::Text(mapped)) => {
            tokens.insert(mapped.clone());
        }
        Some(BetaNode::Flag(_) | BetaNode::Number(_) | BetaNode::List(_) | BetaNode::Map(_)) => {
            return Err(BetaFilterError::MappingValue {
                provider: provider.to_owned(),
                token: token.to_owned(),
            });
        }
    }
    Ok(())
}

pub fn filter_header_betas(
    headers: &mut Headers,
    matrix: &BetaMatrix,
    provider: &str,
) -> Result<Vec<DroppedBeta>, BetaFilterError> {
    let Some(existing) = headers.get("anthropic-beta") else {
        return Ok(Vec::new());
    };
    let tokens: Vec<String> = existing
        .split(',')
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(str::to_owned)
        .collect();
    let filtered = filter_beta_tokens(matrix, provider, &tokens)?;
    if filtered.tokens.is_empty() {
        headers.shift_remove("anthropic-beta");
    } else {
        headers.insert("anthropic-beta".to_owned(), filtered.tokens.join(","));
    }
    Ok(filtered.dropped)
}

pub fn filter_request_betas(
    headers: &mut Headers,
    body: &mut IndexMap<String, Json>,
    matrix: &BetaMatrix,
    provider: &str,
) -> Result<Vec<DroppedBeta>, BetaFilterError> {
    let mut dropped = filter_header_betas(headers, matrix, provider)?;
    let Some(existing) = body.get("anthropic_beta") else {
        return Ok(dropped);
    };
    let Some(tokens) = body_beta_tokens(existing)? else {
        return Ok(dropped);
    };
    let filtered = filter_beta_tokens(matrix, provider, &tokens)?;
    dropped.extend(filtered.dropped);
    if filtered.tokens.is_empty() {
        body.shift_remove("anthropic_beta");
        return Ok(dropped);
    }
    let list = Json::List(filtered.tokens.into_iter().map(Json::Text).collect());
    body.insert("anthropic_beta".to_owned(), list);
    Ok(dropped)
}

// LiteLLM keeps a falsey body list as sent; only a nonempty list is filtered and then possibly removed
fn body_beta_tokens(value: &Json) -> Result<Option<Vec<String>>, BetaFilterError> {
    match value {
        Json::Null => Ok(None),
        Json::List(items) if items.is_empty() => Ok(None),
        Json::List(items) => {
            let mut tokens = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(token) = item else {
                    return Err(BetaFilterError::Field {
                        field: "anthropic_beta",
                        expected: "a list of strings",
                    });
                };
                tokens.push(token.clone());
            }
            Ok(Some(tokens))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(BetaFilterError::Field {
                field: "anthropic_beta",
                expected: "a list of strings",
            })
        }
    }
}
