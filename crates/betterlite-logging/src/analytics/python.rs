use serde::Serialize;
use serde_json::ser::{CharEscape, CompactFormatter, Formatter, Serializer};
use std::io::{self, Write};
use std::string::FromUtf8Error;
use std::{fmt, mem};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("cannot format analytics parameter")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("formatted analytics parameter is not UTF-8")]
    Utf8 {
        #[source]
        source: FromUtf8Error,
    },
}

pub fn repr<Value>(value: &Value) -> Result<String, FormatError>
where
    Value: Serialize,
{
    let mut output = Vec::new();
    let formatter = Python { string: Vec::new() };
    value
        .serialize(&mut Serializer::with_formatter(&mut output, formatter))
        .map_err(|source| FormatError::Serialize { source })?;
    String::from_utf8(output).map_err(|source| FormatError::Utf8 { source })
}

struct Python {
    string: Vec<u8>,
}

impl Formatter for Python {
    fn write_null<Out>(&mut self, writer: &mut Out) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        writer.write_all(b"None")
    }

    fn write_bool<Out>(&mut self, writer: &mut Out, value: bool) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        let literal: &[u8] = if value { b"True" } else { b"False" };
        writer.write_all(literal)
    }

    fn begin_string<Out>(&mut self, _: &mut Out) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        CompactFormatter.begin_string(&mut self.string)
    }

    fn write_string_fragment<Out>(&mut self, _: &mut Out, fragment: &str) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        self.string.write_all(fragment.as_bytes())
    }

    fn write_char_escape<Out>(&mut self, _: &mut Out, char_escape: CharEscape) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        CompactFormatter.write_char_escape(&mut self.string, char_escape)
    }

    fn end_string<Out>(&mut self, writer: &mut Out) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        self.string.push(b'"');
        let text = serde_json::from_slice::<String>(&mem::take(&mut self.string))?;
        writer.write_all(quoted(&text)?.as_bytes())
    }

    fn begin_array_value<Out>(&mut self, writer: &mut Out, first: bool) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<Out>(&mut self, writer: &mut Out, first: bool) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        self.begin_array_value(writer, first)
    }

    fn begin_object_value<Out>(&mut self, writer: &mut Out) -> io::Result<()>
    where
        Out: Write + ?Sized,
    {
        writer.write_all(b": ")
    }
}

fn quoted(text: &str) -> io::Result<String> {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut output = String::from(quote);
    for character in text.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\\' => output.push_str("\\\\"),
            other if other == quote => output.extend(['\\', quote]),
            other if other.is_control() => {
                fmt::write(&mut output, format_args!("\\x{:02x}", u32::from(other)))
                    .map_err(io::Error::other)?;
            }
            other => output.push(other),
        }
    }
    output.push(quote);
    Ok(output)
}
