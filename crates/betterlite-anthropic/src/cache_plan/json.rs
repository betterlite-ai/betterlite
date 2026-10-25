use std::io;
use std::ops::Range;

use serde::Serialize;
use serde_json::Deserializer;
use serde_json::ser::{Formatter, Serializer};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

const FIXED_EXPONENTS: Range<i32> = -4_i32..16_i32;

struct PythonFloats;

// LiteLLM hashes json.dumps output, and Python writes floats with repr: 1e-05, 1.5e+16, 100.0
impl Formatter for PythonFloats {
    fn write_f64<Writer>(&mut self, writer: &mut Writer, value: f64) -> io::Result<()>
    where
        Writer: ?Sized + io::Write,
    {
        let scientific = format!("{value:e}");
        let (mantissa, power) = scientific
            .split_once('e')
            .ok_or_else(|| io::Error::other("a float printed without an exponent"))?;
        let exponent: i32 = power.parse().map_err(io::Error::other)?;
        if FIXED_EXPONENTS.contains(&exponent) {
            let fixed = value.to_string();
            let point = if fixed.contains('.') { "" } else { ".0" };
            write!(writer, "{fixed}{point}")
        } else {
            let sign = if exponent < 0 { '-' } else { '+' };
            write!(writer, "{mantissa}e{sign}{:02}", exponent.unsigned_abs())
        }
    }
}

pub(super) fn compact(raw: &RawValue) -> Result<Box<RawValue>, serde_json::Error> {
    let mut text = Vec::new();
    serde_transcode::transcode(
        &mut Deserializer::from_str(raw.get()),
        &mut Serializer::with_formatter(&mut text, PythonFloats),
    )?;
    serde_json::from_slice(&text)
}

pub(super) fn digest<Value>(value: &Value) -> Result<String, serde_json::Error>
where
    Value: Serialize,
{
    let mut hasher = Sha256::new();
    serde_json::to_writer(&mut hasher, value)?;
    Ok(format!("{:x}", hasher.finalize()))
}
