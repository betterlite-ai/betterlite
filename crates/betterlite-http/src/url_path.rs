use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

use crate::error::UrlPathError;

const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub fn encode_url_path_segment(value: &str) -> Result<String, UrlPathError> {
    let field_name = "path parameter";
    encode_segment(value, field_name)
}

pub fn encode_segment(value: &str, field_name: &str) -> Result<String, UrlPathError> {
    validate_segment(value, field_name)?;
    Ok(utf8_percent_encode(value, PATH_SEGMENT).to_string())
}

pub fn encode_url_path_segments(value: &str) -> Result<String, UrlPathError> {
    let field_name = "path";
    encode_segments(value, field_name)
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
