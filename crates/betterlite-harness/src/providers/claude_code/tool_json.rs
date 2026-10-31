use std::io;

use serde::Serialize as _;
use serde_json::ser::Formatter;

use super::WireValue;

struct ToolJson;

impl Formatter for ToolJson {
    fn begin_array_value<Writer>(&mut self, writer: &mut Writer, first: bool) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<Writer>(&mut self, writer: &mut Writer, first: bool) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        self.begin_array_value(writer, first)
    }

    fn begin_object_value<Writer>(&mut self, writer: &mut Writer) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}

pub(super) fn encode(value: &WireValue) -> String {
    let mut bytes = Vec::new();
    if value
        .serialize(&mut serde_json::Serializer::with_formatter(
            &mut bytes, ToolJson,
        ))
        .is_err()
    {
        return String::new();
    }
    String::from_utf8(bytes).unwrap_or_default()
}
