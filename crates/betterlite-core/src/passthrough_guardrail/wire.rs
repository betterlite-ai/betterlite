use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use super::errors::PassthroughPayloadError;
use super::settings::RequestData;

const PRIVATE_NUMBER_KEY: &str = "$serde_json::private::Number";

// serde_json's arbitrary_precision hands an untagged enum every float as {"$serde_json::private::Number": "0.7"}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireValue {
    PrivateNumber(PrivateNumber),
    Object(BTreeMap<String, Self>),
    Null,
    Boolean(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
    Text(String),
    Array(Vec<Self>),
}

#[derive(Debug, Deserialize)]
struct PrivateNumber {
    #[serde(rename = "$serde_json::private::Number")]
    number: String,
    #[serde(flatten)]
    extra: BTreeMap<String, WireValue>,
}

impl TryFrom<WireValue> for GuardrailValue {
    type Error = PassthroughPayloadError;

    fn try_from(wire: WireValue) -> Result<Self, Self::Error> {
        Ok(match wire {
            WireValue::PrivateNumber(private) => private_number(private)?,
            WireValue::Object(entries) => Self::Object(convert_entries(entries)?),
            WireValue::Null => Self::Null,
            WireValue::Boolean(flag) => Self::Boolean(flag),
            WireValue::SignedInteger(number) => Self::SignedInteger(number),
            WireValue::UnsignedInteger(number) => Self::UnsignedInteger(number),
            WireValue::Decimal(number) => Self::Decimal(number),
            WireValue::Text(text) => Self::Text(text),
            WireValue::Array(children) => Self::Array(
                children
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

fn convert_entries(
    entries: BTreeMap<String, WireValue>,
) -> Result<BTreeMap<String, GuardrailValue>, PassthroughPayloadError> {
    entries
        .into_iter()
        .map(|(key, child)| Ok((key, GuardrailValue::try_from(child)?)))
        .collect()
}

fn is_integer_literal(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|digit| digit.is_ascii_digit())
}

// a lone private key is a number; with siblings or text that is no number it is the caller's own object
fn private_number(private: PrivateNumber) -> Result<GuardrailValue, PassthroughPayloadError> {
    let PrivateNumber {
        number: text,
        extra,
    } = private;
    if extra.is_empty() {
        if let Ok(number) = text.parse::<i64>() {
            return Ok(GuardrailValue::SignedInteger(number));
        }
        if let Ok(number) = text.parse::<u64>() {
            return Ok(GuardrailValue::UnsignedInteger(number));
        }
        if is_integer_literal(&text) {
            return Err(PassthroughPayloadError::IntegerOutOfRange { text });
        }
        if let Ok(number) = text.parse::<f64>() {
            return Ok(GuardrailValue::Decimal(number));
        }
    }
    let mut entries = extra;
    entries.insert(PRIVATE_NUMBER_KEY.to_owned(), WireValue::Text(text));
    Ok(GuardrailValue::Object(convert_entries(entries)?))
}

// GuardrailValue's own Deserialize turns a non-integer number into the private-key object under arbitrary_precision
pub fn parse_request_data(source: &str) -> Result<RequestData, PassthroughPayloadError> {
    let wire = serde_json::from_str::<BTreeMap<String, WireValue>>(source)
        .map_err(PassthroughPayloadError::Parse)?;
    convert_entries(wire)
}
