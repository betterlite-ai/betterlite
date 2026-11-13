use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use thiserror::Error;

// urllib.parse.quote(part, safe="") leaves only letters, digits and _.-~ alone
const QUOTE_SAFE_NONE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Debug, Error)]
pub enum PathError {
    #[error("Invalid file path {path:?}: contains URL special characters")]
    SpecialCharacters { path: String },
    #[error("Invalid file path {path:?}: path traversal detected")]
    Traversal { path: String },
}

pub fn sanitize_file_path(file_path: &str) -> Result<String, PathError> {
    if file_path.contains(['#', '?']) {
        return Err(PathError::SpecialCharacters {
            path: file_path.to_owned(),
        });
    }
    let segments: Vec<&str> = file_path.split('/').collect();
    if segments.contains(&"..") {
        return Err(PathError::Traversal {
            path: file_path.to_owned(),
        });
    }
    let quoted: Vec<String> = segments
        .iter()
        .map(|segment| utf8_percent_encode(segment, &QUOTE_SAFE_NONE).to_string())
        .collect();
    Ok(quoted.join("/"))
}
