use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::ser::Error as _;
use serde_json::value::RawValue;

use super::wire::{Arguments, Input};

pub(super) fn normalize(raw: &Arguments, responses: bool) -> Input {
    let Arguments::Present(value) = raw else {
        return Input::default();
    };
    if let Ok(text) = serde_json::from_str::<String>(value.get()) {
        return serde_json::from_str(&text).unwrap_or_else(|_| {
            tracing::warn!("Cannot parse search function arguments");
            Input::default()
        });
    }
    match serde_json::from_str::<Input>(value.get()) {
        Ok(input @ Input::Object(_)) => input,
        Ok(Input::Other(payload))
            if !responses && !matches!(payload.get(), "null" | "false" | "0" | "[]") =>
        {
            Input::Other(payload)
        }
        Ok(Input::Other(_)) | Err(_) => Input::default(),
    }
}

pub(super) fn wire_text(input: &Input) -> Result<String, serde_json::Error> {
    match input {
        Input::Object(fields) => object_text(fields, false),
        Input::Other(raw) => {
            if let Ok(text) = serde_json::from_str::<String>(raw.get()) {
                return Ok(text);
            }
            text(raw, true)
        }
    }
}

fn object_text(
    fields: &IndexMap<String, Box<RawValue>>,
    python: bool,
) -> Result<String, serde_json::Error> {
    let entries = fields
        .iter()
        .map(|(key, value)| Ok(format!("{}: {}", quote(key, python)?, text(value, python)?)))
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    Ok(format!("{{{}}}", entries.join(", ")))
}

pub(super) fn text(raw: &RawValue, python: bool) -> Result<String, serde_json::Error> {
    match raw.get().as_bytes().first() {
        Some(b'{') => object_text(&serde_json::from_str(raw.get())?, python),
        Some(b'[') => {
            let items: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            let values = items
                .iter()
                .map(|item| text(item, python))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", values.join(", ")))
        }
        Some(b'"') => quote(&serde_json::from_str::<String>(raw.get())?, python),
        Some(b'n') if python => Ok("None".to_owned()),
        Some(b't') if python => Ok("True".to_owned()),
        Some(b'f') if python => Ok("False".to_owned()),
        _ => Ok(raw.get().to_owned()),
    }
}

pub(super) fn quote(value: &str, python: bool) -> Result<String, serde_json::Error> {
    let quoted = serde_json::to_string(value)?;
    if python {
        let delimiter = if value.contains('\'') && !value.contains('"') {
            '"'
        } else {
            '\''
        };
        let escaped = value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
            .replace(delimiter, &format!("\\{delimiter}"));
        return Ok(format!("{delimiter}{escaped}{delimiter}"));
    }
    let mut ascii = String::new();
    for character in quoted.chars() {
        if character.is_ascii() && character != '\u{7f}' {
            ascii.push(character);
        } else {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                write!(ascii, "\\u{unit:04x}").map_err(serde_json::Error::custom)?;
            }
        }
    }
    Ok(ascii)
}
