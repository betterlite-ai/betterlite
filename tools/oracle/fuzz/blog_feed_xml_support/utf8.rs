use std::borrow::Cow;
use std::str;

pub(crate) fn lossy_utf8(bytes: &[u8]) -> Cow<'_, str> {
    str::from_utf8(bytes).map_or_else(
        |_| Cow::Owned(replace_invalid_sequences(bytes)),
        Cow::Borrowed,
    )
}

fn replace_invalid_sequences(mut remaining: &[u8]) -> String {
    let mut output = String::new();
    loop {
        if remaining.is_empty() {
            break;
        }
        match str::from_utf8(remaining) {
            Ok(text) => {
                output.push_str(text);
                break;
            }
            Err(error) => {
                let valid_length = error.valid_up_to();
                let Some(valid_bytes) = remaining.get(..valid_length) else {
                    break;
                };
                if let Ok(valid_text) = str::from_utf8(valid_bytes) {
                    output.push_str(valid_text);
                }
                let Some(invalid_bytes) = remaining.get(valid_length..) else {
                    break;
                };
                if invalid_bytes.is_empty() {
                    break;
                }
                let invalid_length = error.error_len().unwrap_or(invalid_bytes.len());
                if invalid_length == 0 {
                    break;
                }
                output.push('\u{FFFD}');
                let Some(rest) = invalid_bytes.get(invalid_length..) else {
                    break;
                };
                remaining = rest;
            }
        }
    }
    output
}
