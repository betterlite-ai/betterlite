use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::error::UrlPathError;

// Python's quote(value, safe="") keeps only the RFC 3986 unreserved characters.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub fn encode_segment(value: &str, field_name: &str) -> Result<String, UrlPathError> {
    validate_segment(value, field_name)?;
    Ok(utf8_percent_encode(value, PATH_SEGMENT).to_string())
}

pub fn encode_segments(value: &str, field_name: &str) -> Result<String, UrlPathError> {
    if value.is_empty() {
        return Err(UrlPathError::Required {
            field_name: field_name.to_owned(),
        });
    }
    value
        .split('/')
        .map(|segment| encode_segment(segment, field_name))
        .collect::<Result<Vec<_>, _>>()
        .map(|segments| segments.join("/"))
}

fn validate_segment(value: &str, field_name: &str) -> Result<(), UrlPathError> {
    if value.is_empty() {
        return Err(UrlPathError::Required {
            field_name: field_name.to_owned(),
        });
    }
    if value == "." || value == ".." {
        return Err(UrlPathError::DotSegment {
            field_name: field_name.to_owned(),
        });
    }
    Ok(())
}

mod tests;
