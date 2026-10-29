use std::io;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::ser::Formatter;

// json.loads keeps keys in written order with the last duplicate's value, and json.dumps writes them back so
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum OrderedJson {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

// json.dumps with its default separators and ensure_ascii, the text LiteLLM puts in prompts and results
struct PythonJson;

impl Formatter for PythonJson {
    fn begin_array_value<Writer>(&mut self, writer: &mut Writer, first: bool) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(if first { b"" } else { b", " })
    }

    fn begin_object_key<Writer>(&mut self, writer: &mut Writer, first: bool) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(if first { b"" } else { b", " })
    }

    fn begin_object_value<Writer>(&mut self, writer: &mut Writer) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }

    fn write_string_fragment<Writer>(
        &mut self,
        writer: &mut Writer,
        fragment: &str,
    ) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        for character in fragment.chars() {
            if character.is_ascii() && !character.is_ascii_control() {
                write!(writer, "{character}")?;
                continue;
            }
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                write!(writer, "\\u{unit:04x}")?;
            }
        }
        Ok(())
    }

    fn write_number_str<Writer>(&mut self, writer: &mut Writer, value: &str) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(python_number(value)?.as_bytes())
    }

    fn write_f64<Writer>(&mut self, writer: &mut Writer, value: f64) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        writer.write_all(python_float(value).as_bytes())
    }
}

fn python_number(text: &str) -> io::Result<String> {
    // json.loads makes a float of a number with a fraction or an exponent and an int of any other
    if text.contains(['.', 'e', 'E']) {
        return text.parse().map(python_float).map_err(io::Error::other);
    }
    Ok(if text == "-0" { "0" } else { text }.to_owned())
}

fn python_float(value: f64) -> String {
    if value.is_infinite() {
        let infinity = if value.is_sign_positive() {
            "Infinity"
        } else {
            "-Infinity"
        };
        return infinity.to_owned();
    }
    // Rust and Python switch to exponents at the same magnitudes; Python signs and pads the exponent
    let shortest = format!("{value:?}");
    let Some((mantissa, exponent)) = shortest.split_once('e') else {
        return shortest;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(("+", exponent), |digits| ("-", digits));
    format!("{mantissa}e{sign}{digits:0>2}")
}

pub(crate) fn python_json_text<Document>(document: &Document) -> Result<String, serde_json::Error>
where
    Document: Serialize,
{
    let mut output = Vec::new();
    document.serialize(&mut serde_json::Serializer::with_formatter(
        &mut output,
        PythonJson,
    ))?;
    // PythonJson escapes everything outside ASCII, so every byte is one character
    Ok(output.into_iter().map(char::from).collect())
}
