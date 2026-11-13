use std::io;

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JsonError {
    #[error("the estimator value could not be serialized to JSON")]
    Serialize(#[source] serde_json::Error),
    #[error("the serialized estimator value is not UTF-8")]
    Utf8(#[source] std::string::FromUtf8Error),
}

// LiteLLM hashes and sends json.dumps output, whose default separators carry one space after each comma and colon
struct PythonFormatter;

impl serde_json::ser::Formatter for PythonFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            return Ok(());
        }
        writer.write_all(b", ")
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            return Ok(());
        }
        writer.write_all(b", ")
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}

pub fn python_json<T>(value: &T) -> Result<String, JsonError>
where
    T: ?Sized + Serialize,
{
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, PythonFormatter);
    value
        .serialize(&mut serializer)
        .map_err(|source| JsonError::Serialize { source })?;
    String::from_utf8(bytes).map_err(|source| JsonError::Utf8 { source })
}
