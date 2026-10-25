use std::io::{self, Write};
use std::string::FromUtf8Error;

use serde::Serialize as _;
use serde_json::Serializer;
use serde_json::ser::Formatter;

use crate::decisions::DecisionsJson;

#[derive(Debug, thiserror::Error)]
pub enum TextError {
    #[error("the decision JSON could not be written")]
    Encode(#[source] serde_json::Error),
    #[error("the decision JSON was not UTF-8")]
    Utf8(#[source] FromUtf8Error),
}

pub fn decisions_text(value: &DecisionsJson) -> Result<String, TextError> {
    if let DecisionsJson::Text(text) = value {
        return Ok(text.clone());
    }
    let mut serializer = Serializer::with_formatter(Vec::new(), PythonFormatter);
    value
        .serialize(&mut serializer)
        .map_err(TextError::Encode)?;
    String::from_utf8(serializer.into_inner()).map_err(TextError::Utf8)
}

// LiteLLM sends json.dumps output to the provider: ", " and ": " separators and every non-ASCII character escaped
struct PythonFormatter;

impl Formatter for PythonFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        self.begin_array_value(writer, first)
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        writer.write_all(b": ")
    }

    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        let mut units = [0; 2];
        for character in fragment.chars() {
            match character {
                ' '..='~' => write!(writer, "{character}")?,
                _ => character
                    .encode_utf16(&mut units)
                    .iter()
                    .try_for_each(|unit| write!(writer, "\\u{unit:04x}"))?,
            }
        }
        Ok(())
    }
}
