use std::io::{self, Write};

use betterlite_prompts::arguments::json::Json;
use serde::Serialize as _;
use serde_json::Serializer;
use serde_json::ser::Formatter;

use super::error::LangGraphFailure;

// json.dumps with its default separators (", " and ": ") and every non-ASCII character escaped
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

pub fn dumps(value: &Json) -> Result<String, LangGraphFailure> {
    let mut serializer = Serializer::with_formatter(Vec::new(), PythonFormatter);
    value
        .serialize(&mut serializer)
        .map_err(|source| LangGraphFailure::Dumps { source })?;
    // every non-ASCII character was escaped, so each byte is one character
    Ok(serializer
        .into_inner()
        .into_iter()
        .map(char::from)
        .collect())
}
