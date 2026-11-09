use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StreamEvent {
    pub generated_text: String,
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("the NLP Cloud stream line is not a generated_text event")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM yields a buffer once its NUL separators are gone; an empty line is skipped rather than ending the stream
#[must_use]
pub fn clean_and_iterate_chunks<'line, Lines>(lines: Lines) -> Vec<String>
where
    Lines: IntoIterator<Item = &'line str>,
{
    let mut buffer = String::new();
    let mut chunks = Vec::new();
    for line in lines {
        buffer.push_str(line);
        if !buffer.contains('\0') {
            continue;
        }
        let cleaned = buffer.replace('\0', "");
        buffer.clear();
        if !cleaned.is_empty() {
            chunks.push(cleaned);
        }
    }
    if !buffer.is_empty() {
        chunks.push(buffer);
    }
    chunks
}
