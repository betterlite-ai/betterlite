use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub enum HeadersInput<'source> {
    None,
    Text(&'source str),
    Map(&'source BTreeMap<String, String>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HeadersError {
    #[error("the OpenTelemetry header entry at position {position} has no equals sign")]
    MissingSeparator { position: usize },
}

// The error names the position only: the text it came from may hold a credential.
pub fn headers_dictionary(
    input: HeadersInput<'_>,
) -> Result<BTreeMap<String, String>, HeadersError> {
    let mut headers = BTreeMap::new();
    match input {
        HeadersInput::None => {}
        HeadersInput::Map(source) => headers = source.clone(),
        HeadersInput::Text(text) => {
            if text.is_empty() {
                return Ok(headers);
            }
            for (index, part) in text.split(',').enumerate() {
                let Some((name, value)) = part.split_once('=') else {
                    return Err(HeadersError::MissingSeparator {
                        position: index.saturating_add(1),
                    });
                };
                headers.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    Ok(headers)
}

#[path = "headers/tests.rs"]
mod tests;
