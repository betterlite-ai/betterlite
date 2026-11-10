use sha2::{Digest as _, Sha256};

use crate::anthropic_results::ids::replace_invalid_id_chars;

pub const MAX_TOOL_USE_ID_LEN: usize = 64;
const HASH_LEN: usize = 8;
const MAX_KEPT_LEN: usize = MAX_TOOL_USE_ID_LEN
    .saturating_sub(HASH_LEN)
    .saturating_sub(1);

// Bedrock Converse takes [a-zA-Z0-9_.:-]+ up to 64 chars; a rewritten id keeps a hash of the original so two ids stay distinct
#[must_use]
pub fn sanitize_bedrock_tool_use_id(tool_use_id: &str) -> String {
    let sanitized = replace_invalid_id_chars(tool_use_id, |candidate| {
        !(candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '.' | ':' | '-'))
    });
    if sanitized == tool_use_id && sanitized.chars().count() <= MAX_TOOL_USE_ID_LEN {
        return sanitized;
    }
    let digest = hex::encode(Sha256::digest(tool_use_id.as_bytes()));
    let hash = digest.get(..HASH_LEN).unwrap_or(&digest);
    let kept: String = sanitized.chars().take(MAX_KEPT_LEN).collect();
    format!("{kept}_{hash}")
}
