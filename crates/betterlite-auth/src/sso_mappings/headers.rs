use std::str;

use indexmap::IndexMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeaderError {
    #[error("the SSO headers are not UTF-8 text")]
    NotText,
    #[error("the SSO header {header:?} has no equals sign")]
    MissingEquals { header: String },
    #[error("the SSO header {header:?} has more than one equals sign")]
    TooManyEquals { header: String },
}

// str.strip() also drops the four ASCII separator controls, which char::is_whitespace does not.
const fn is_space(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        _ => character.is_whitespace(),
    }
}

// Python's header parser splits each segment on every equals sign and unpacks exactly two parts
pub fn decode_generic_headers(raw: Option<&[u8]>) -> Result<IndexMap<String, String>, HeaderError> {
    let Some(bytes) = raw else {
        return Ok(IndexMap::new());
    };
    let Ok(text) = str::from_utf8(bytes) else {
        return Err(HeaderError::NotText);
    };
    let mut headers = IndexMap::new();
    for segment in text.split(',') {
        let header = segment.trim_matches(is_space);
        if header.is_empty() {
            continue;
        }
        let Some((key, value)) = header.split_once('=') else {
            return Err(HeaderError::MissingEquals {
                header: header.to_owned(),
            });
        };
        if value.contains('=') {
            return Err(HeaderError::TooManyEquals {
                header: header.to_owned(),
            });
        }
        headers.insert(key.to_owned(), value.to_owned());
    }
    Ok(headers)
}
