use indexmap::IndexMap;
use serde_json::value::RawValue;

// Python's frame is json.dumps text, including spaces, ASCII escapes and insertion order.
pub(super) fn format(raw: &RawValue) -> Result<String, serde_json::Error> {
    let text = raw.get();
    match text.as_bytes().first() {
        Some(b'{') => {
            let object: IndexMap<String, &RawValue> = serde_json::from_str(text)?;
            let fields = object
                .iter()
                .map(|(key, value)| Ok(format!("{}: {}", quoted(key)?, format(value)?)))
                .collect::<Result<Vec<_>, serde_json::Error>>()?;
            Ok(format!("{{{}}}", fields.join(", ")))
        }
        Some(b'[') => {
            let items: Vec<&RawValue> = serde_json::from_str(text)?;
            let values = items
                .into_iter()
                .map(format)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", values.join(", ")))
        }
        Some(b'"') => quoted(&serde_json::from_str::<String>(text)?),
        Some(b'-' | b'0'..=b'9') => number_text(text),
        Some(_) | None => Ok(text.to_owned()),
    }
}

fn quoted(text: &str) -> Result<String, serde_json::Error> {
    let escaped = serde_json::to_string(text)?;
    let ascii = escaped
        .encode_utf16()
        .map(|unit| match unit {
            0x0020..=0x007e => String::from_utf16_lossy(&[unit]),
            _ => format!("\\u{unit:04x}"),
        })
        .collect();
    Ok(ascii)
}

fn number_text(text: &str) -> Result<String, serde_json::Error> {
    if !text.contains(['.', 'e', 'E']) {
        return Ok(if text == "-0" { "0" } else { text }.to_owned());
    }
    let number: serde_json::Number = serde_json::from_str(text)?;
    let rendered = number
        .as_f64()
        .map_or_else(|| number.to_string(), |value| format!("{value:?}"));
    match rendered.split_once('e') {
        Some((mantissa, power)) => {
            let (sign, digits) = power
                .strip_prefix('-')
                .map_or(("+", power), |digits| ("-", digits));
            Ok(format!("{mantissa}e{sign}{digits:0>2}"))
        }
        None => Ok(rendered),
    }
}
