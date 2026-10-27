mod tests;

use thiserror::Error;

#[derive(Debug, Eq, Error, PartialEq)]
pub enum ReplaceError {
    #[error("Error: old_string cannot be empty. Provide the exact text to replace.")]
    EmptyOld,
    #[error(
        "Error: old_string ends with a newline, but the file does not end with a newline. Retry with the trailing newline removed from old_string (and from new_string if it also ends with a newline)."
    )]
    TrailingNewline,
    #[error(
        "Error: old_string ends with a newline, but the file does not end with a newline. With the trailing newline removed, old_string would appear {count} times in the file. Retry with the trailing newline removed and add surrounding context so the match is unique."
    )]
    TrailingNewlineAmbiguous { count: usize },
    #[error("Error: String not found in file: '{old}'")]
    NotFound { old: String },
    #[error(
        "Error: String '{old}' appears {count} times in file. Use replace_all=True to replace all instances, or provide a more specific string with surrounding context."
    )]
    Ambiguous { old: String, count: usize },
    #[error("Error: replacement result exceeds {limit} bytes")]
    ResultTooLarge { limit: usize },
}

#[derive(Debug, Eq, PartialEq)]
pub struct Replaced {
    pub content: String,
    pub occurrences: usize,
}

pub fn replace_occurrences(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Replaced, ReplaceError> {
    if old.is_empty() {
        return Err(ReplaceError::EmptyOld);
    }
    let occurrences = content.matches(old).count();
    if occurrences == 0 {
        return Err(missing_match(content, old));
    }
    if occurrences > 1 && !replace_all {
        return Err(ReplaceError::Ambiguous {
            old: old.to_owned(),
            count: occurrences,
        });
    }
    let removed_bytes = occurrences
        .checked_mul(old.len())
        .ok_or(ReplaceError::ResultTooLarge {
            limit: super::READ_CAP_BYTES,
        })?;
    let added_bytes = occurrences
        .checked_mul(new.len())
        .ok_or(ReplaceError::ResultTooLarge {
            limit: super::READ_CAP_BYTES,
        })?;
    let result_len = content
        .len()
        .checked_sub(removed_bytes)
        .and_then(|length| length.checked_add(added_bytes))
        .ok_or(ReplaceError::ResultTooLarge {
            limit: super::READ_CAP_BYTES,
        })?;
    if result_len > super::READ_CAP_BYTES {
        return Err(ReplaceError::ResultTooLarge {
            limit: super::READ_CAP_BYTES,
        });
    }
    Ok(Replaced {
        content: content.replace(old, new),
        occurrences,
    })
}

fn missing_match(content: &str, old: &str) -> ReplaceError {
    match old.strip_suffix('\n') {
        Some(stripped) if !stripped.is_empty() && content.ends_with(stripped) => {
            match content.matches(stripped).count() {
                1 => ReplaceError::TrailingNewline,
                count => ReplaceError::TrailingNewlineAmbiguous { count },
            }
        }
        _ => ReplaceError::NotFound {
            old: old.to_owned(),
        },
    }
}
