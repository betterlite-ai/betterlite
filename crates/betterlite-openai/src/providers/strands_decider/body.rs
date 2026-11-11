use std::io::{self, Write};

use serde::Serialize as _;
use serde_json::Serializer;
use serde_json::ser::Formatter;

use crate::decisions::transformation::wire::ProviderRequestWire;

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("the decision request body could not be written")]
    Encode(#[source] serde_json::Error),
}

pub fn request_body(body: &ProviderRequestWire) -> Result<Vec<u8>, BodyError> {
    let mut serializer = Serializer::with_formatter(Vec::new(), PythonFormatter);
    body.serialize(&mut serializer).map_err(BodyError::Encode)?;
    Ok(serializer.into_inner())
}

// LiteLLM signs json.dumps output, so the separators keep the body bytes it would produce
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
}
