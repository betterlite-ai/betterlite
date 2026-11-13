use strfmt::{FmtError, strfmt_map};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormatError {
    #[error("'{0}'")]
    Key(String),
    #[error("Replacement index {0} out of range for positional args tuple")]
    Position(usize),
    #[error("{0}")]
    Invalid(String),
}

pub fn format_reason(template: &str, reason: &str) -> Result<String, FormatError> {
    let rendered = strfmt_map(template, |mut field| {
        let name = field.key.split('!').next().unwrap_or_default();
        if name != "reason" {
            return Err(FmtError::KeyError(name.to_owned()));
        }
        match conversion(field.key) {
            "" | "s" => field.str(reason),
            other => Err(FmtError::Invalid(format!(
                "Unknown conversion specifier {other}"
            ))),
        }
    });
    rendered.map_err(field_error)
}

pub fn uses_reason_placeholder(template: &str) -> Result<bool, FormatError> {
    let mut found = false;
    let parsed = strfmt_map(template, |field| {
        if field.key.split('!').next() == Some("reason") {
            found = true;
        }
        Ok(())
    });
    parsed.map_err(field_error)?;
    Ok(found)
}

// strfmt keeps the conversion inside the key, while Python's parse splits it off first
fn conversion(key: &str) -> &str {
    match key.split_once('!') {
        Some((_name, conversion)) => conversion,
        None => "",
    }
}

fn field_error(error: FmtError) -> FormatError {
    match error {
        FmtError::KeyError(name) => match name.parse::<usize>() {
            Ok(index) => FormatError::Position(index),
            Err(_not_a_number) => FormatError::Key(name),
        },
        FmtError::TypeError(text) | FmtError::Invalid(text) => FormatError::Invalid(text),
    }
}
