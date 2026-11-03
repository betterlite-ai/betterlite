use std::cmp::Reverse;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::types::MCPServer;

const BASE62_ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE52_ALPHA_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE62_LEN: u64 = 62;
const BASE52_LEN: u64 = 52;
const SHORT_PREFIX_LENGTH: usize = 3;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrefixError {
    #[error("server_id must not be empty to compute a short prefix")]
    EmptyServerId,
    #[error("computed alphabet index does not fit the target platform's usize")]
    IndexOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixPolicy {
    pub separator: String,
    pub short_prefix_enabled: bool,
}

impl Default for PrefixPolicy {
    fn default() -> Self {
        Self {
            separator: "-".to_owned(),
            short_prefix_enabled: false,
        }
    }
}

pub(super) fn compute_short_server_prefix(
    server_id: &str,
    attempt: u32,
) -> Result<String, PrefixError> {
    if server_id.is_empty() {
        return Err(PrefixError::EmptyServerId);
    }
    let seed = if attempt == 0 {
        server_id.to_owned()
    } else {
        format!("{server_id}#{attempt}")
    };
    let digest = Sha256::digest(seed.as_bytes());
    let mut folded = [0_u8; 8];
    let digest_prefix = digest.get(..8).ok_or(PrefixError::IndexOverflow)?;
    folded.copy_from_slice(digest_prefix);
    let mut value = u64::from_be_bytes(folded);

    let mut chars = Vec::with_capacity(SHORT_PREFIX_LENGTH);
    for position in 0..SHORT_PREFIX_LENGTH {
        let alphabet = if position == SHORT_PREFIX_LENGTH - 1 {
            BASE52_ALPHA_ALPHABET
        } else {
            BASE62_ALPHABET
        };
        let len = if position == SHORT_PREFIX_LENGTH - 1 {
            BASE52_LEN
        } else {
            BASE62_LEN
        };
        let alphabet_index = value.checked_rem(len).ok_or(PrefixError::IndexOverflow)?;
        value = value.checked_div(len).ok_or(PrefixError::IndexOverflow)?;
        let alphabet_position = usize::try_from(alphabet_index)
            .map_err(|_conversion_error| PrefixError::IndexOverflow)?;
        let byte = *alphabet
            .get(alphabet_position)
            .ok_or(PrefixError::IndexOverflow)?;
        chars.push(byte);
    }
    chars.reverse();
    String::from_utf8(chars).map_err(|_utf8_error| PrefixError::IndexOverflow)
}

pub(super) fn get_server_prefix(server: &MCPServer, policy: &PrefixPolicy) -> String {
    if policy.short_prefix_enabled {
        if let Some(cached) = server
            .short_prefix
            .as_ref()
            .filter(|value| !value.is_empty())
        {
            return cached.clone();
        }
        if let Ok(computed) = compute_short_server_prefix(&server.server_id, 0) {
            return computed;
        }
    }
    let present = |value: Option<&String>| value.filter(|candidate| !candidate.is_empty()).cloned();
    present(server.alias.as_ref())
        .or_else(|| present(server.server_name.as_ref()))
        .unwrap_or_else(|| server.server_id.clone())
}

pub(super) fn iter_known_server_prefixes(server: &MCPServer, policy: &PrefixPolicy) -> Vec<String> {
    let mut seen = Vec::new();
    let mut push = |candidate_value: Option<String>| {
        if let Some(candidate) = candidate_value.filter(|value| !value.is_empty())
            && !seen.contains(&candidate)
        {
            seen.push(candidate);
        }
    };

    push(Some(get_server_prefix(server, policy)));
    push(server.short_prefix.clone());
    push(compute_short_server_prefix(&server.server_id, 0).ok());
    push(server.alias.clone());
    push(server.server_name.clone());
    push(Some(server.server_id.clone()));
    seen
}

pub(super) fn match_known_server_prefix(
    name: &str,
    known_prefixes: &[String],
    policy: &PrefixPolicy,
) -> Option<(String, String)> {
    let mut candidates: Vec<String> = known_prefixes
        .iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.replace(' ', "_"))
        .collect();
    candidates.sort_by_key(|value| Reverse(value.len()));
    candidates.dedup();

    for candidate in candidates {
        let prefix_with_separator = format!("{candidate}{}", policy.separator);
        if let Some(rest) = name.strip_prefix(prefix_with_separator.as_str()) {
            return Some((candidate, rest.to_owned()));
        }
    }
    None
}

pub(super) fn split_server_prefix_from_name(
    prefixed_name: &str,
    policy: &PrefixPolicy,
) -> (String, String) {
    match prefixed_name.split_once(policy.separator.as_str()) {
        Some((prefix, rest)) => (rest.to_owned(), prefix.to_owned()),
        None => (prefixed_name.to_owned(), String::new()),
    }
}

#[must_use]
pub fn strip_known_server_prefix(
    name: &str,
    server: Option<&MCPServer>,
    policy: &PrefixPolicy,
) -> String {
    let Some(server_ref) = server else {
        return split_server_prefix_from_name(name, policy).0;
    };
    let known_prefixes = iter_known_server_prefixes(server_ref, policy);
    match match_known_server_prefix(name, &known_prefixes, policy) {
        Some((_, bare_name)) => bare_name,
        None => name.to_owned(),
    }
}
