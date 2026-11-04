use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;

use betterlite_config::sections::content_filter_records::BlockedWord;
use serde::Deserialize;
use serde::de::IgnoredAny;

mod entry;
mod tests;

const MAX_FILE_BYTES: usize = 0x0010_0000;

#[derive(Debug, thiserror::Error)]
pub enum WordsError {
    #[error("Blocked words file not found: {path}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("Invalid format: file must contain 'blocked_words' key with list of words")]
    Structure,
    #[error("invalid blocked words YAML: {0}")]
    Yaml(#[from] serde_saphyr::Error),
    #[error("invalid blocked word field: {0}")]
    Field(&'static str),
    #[error("blocked words file exceeds the size limit")]
    TooLarge,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Document {
    Mapping { blocked_words: IgnoredAny },
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct WordFile {
    blocked_words: Vec<entry::Entry>,
}

pub fn decode(bytes: &[u8]) -> Result<Vec<BlockedWord>, WordsError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(WordsError::TooLarge);
    }
    let document: Document = serde_saphyr::from_slice(bytes)?;
    if !matches!(
        document,
        Document::Mapping {
            blocked_words: IgnoredAny
        }
    ) {
        return Err(WordsError::Structure);
    }
    let parsed: WordFile = serde_saphyr::from_slice(bytes)?;
    let mut words = BTreeMap::new();
    for (index, entry) in parsed.blocked_words.into_iter().enumerate() {
        let Some(word) = entry::into_word(entry)? else {
            tracing::warn!(index, "Skipping invalid word entry");
            continue;
        };
        let keyword = word.keyword.to_lowercase();
        if words
            .insert(
                keyword.clone(),
                BlockedWord {
                    keyword,
                    action: word.action,
                    description: word.description,
                },
            )
            .is_some()
        {
            tracing::warn!(index, "Replacing duplicate blocked word");
        }
    }
    Ok(words.into_values().collect())
}

pub fn load(path: &Path) -> Result<Vec<BlockedWord>, WordsError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(0x0010_0001).read_to_end(&mut bytes))
        .map_err(|source| WordsError::Read {
            path: path.display().to_string(),
            source,
        })?;
    decode(&bytes)
}
