use sha2::{Digest as _, Sha256};
use unicode_general_category::{GeneralCategory, get_general_category};

use super::tools::COMPRESR_RETRIEVE_TOOL_NAME;

const HASH_LENGTH: usize = 24;
const DISPLAY_LIMIT: usize = 32;

#[must_use]
pub fn content_hash(text: &str) -> String {
    let mut digest = hex::encode(Sha256::digest(text.as_bytes()));
    digest.truncate(HASH_LENGTH);
    digest
}

// Python's str.isprintable: everything except the Other and Separator categories, and a plain space stays
fn is_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    !matches!(
        get_general_category(character),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
            | GeneralCategory::SpaceSeparator
    )
}

// A model-supplied hash is echoed into logs and the conversation, so it loses control characters and is capped
#[must_use]
pub fn display_hash(hash: &str) -> String {
    let printable: String = hash
        .chars()
        .filter(|character| is_printable(*character))
        .collect();
    if printable.chars().count() <= DISPLAY_LIMIT {
        return printable;
    }
    let head: String = printable.chars().take(DISPLAY_LIMIT).collect();
    format!("{head}\u{2026}")
}

#[must_use]
pub fn recovery_marker(hash: &str) -> String {
    format!(
        "\n\n[compresr hash={hash}: parts of this content were compressed away. If you need the full original, call the {COMPRESR_RETRIEVE_TOOL_NAME} tool with this hash.]"
    )
}
