use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::trusted_network::cidr::{
    CidrRanges, network_from_text, normalize_cidr_ranges, parse_address,
};

pub const MAX_NETWORK_HEADER_BYTES: usize = 0x0010_0000;
pub const MAX_NETWORK_HEADER_DEPTH: usize = 64;

// The header document the service writes and reads back through the one bounded decoder.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_forwarded_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_forwarded_proto: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_proxy_ranges: Option<CidrRanges>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_trusted_proxy_ranges: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the network header document is {size} bytes, over the {limit}-byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("the network header document nests {depth} levels, over the {limit}-level limit")]
    TooDeep { depth: usize, limit: usize },
    #[error("the network header document is not valid")]
    Json(#[source] serde_json::Error),
    #[error("the network header document could not be written")]
    Write(#[source] serde_json::Error),
}

// Outside bytes stop here: the bounds run before any policy function sees the document.
pub fn decode_network_headers(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_NETWORK_HEADER_BYTES {
        return Err(DecodeError::TooLarge {
            size: bytes.len(),
            limit: MAX_NETWORK_HEADER_BYTES,
        });
    }
    let depth = document_depth(bytes);
    if depth > MAX_NETWORK_HEADER_DEPTH {
        return Err(DecodeError::TooDeep {
            depth,
            limit: MAX_NETWORK_HEADER_DEPTH,
        });
    }
    let document: TypedValue = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
    // The policy parses this text again; the fuzzer and the corpus replay reach the same parsers here.
    for range in normalize_cidr_ranges(document.trusted_proxy_ranges.as_ref()) {
        let _range = network_from_text(&range);
    }
    for range in document.mcp_trusted_proxy_ranges.iter().flatten() {
        let _mcp_range = network_from_text(range);
    }
    for hop in document
        .x_forwarded_for
        .iter()
        .flat_map(|header| header.split(','))
    {
        let _hop = parse_address(hop.trim());
    }
    Ok(document)
}

// Brackets inside a JSON string are text, so raw bytes bound the depth before serde reads the value.
fn document_depth(bytes: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for &byte in bytes {
        if quoted {
            match byte {
                b'\\' if !escaped => escaped = true,
                b'"' if !escaped => quoted = false,
                _ => escaped = false,
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth = depth.saturating_add(1);
                    deepest = deepest.max(depth);
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    deepest
}
