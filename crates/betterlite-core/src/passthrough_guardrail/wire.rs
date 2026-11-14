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

impl From<WireValue> for GuardrailValue {
    fn from(wire: WireValue) -> Self {
        match wire {
            WireValue::PrivateNumber(private) => private_number(private),
            WireValue::Object(entries) => Self::Object(
                entries
                    .into_iter()
                    .map(|(key, child)| (key, Self::from(child)))
                    .collect(),
            ),
            WireValue::Null => Self::Null,
            WireValue::Boolean(flag) => Self::Boolean(flag),
            WireValue::SignedInteger(number) => Self::SignedInteger(number),
            WireValue::UnsignedInteger(number) => Self::UnsignedInteger(number),
            WireValue::Decimal(number) => Self::Decimal(number),
            WireValue::Text(text) => Self::Text(text),
            WireValue::Array(children) => {
                Self::Array(children.into_iter().map(Self::from).collect())
            }
        }
    }
}

// a lone private key is a number; with siblings or text that is no number it is the caller's own object
fn private_number(private: PrivateNumber) -> GuardrailValue {
    let PrivateNumber {
        number: text,
        extra,
    } = private;
    if extra.is_empty() {
        if let Ok(number) = text.parse::<i64>() {
            return GuardrailValue::SignedInteger(number);
        }
        if let Ok(number) = text.parse::<u64>() {
            return GuardrailValue::UnsignedInteger(number);
        }
        if let Ok(number) = text.parse::<f64>() {
            return GuardrailValue::Decimal(number);
        }
    }
    let mut entries = extra;
    entries.insert(PRIVATE_NUMBER_KEY.to_owned(), WireValue::Text(text));
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(key, child)| (key, GuardrailValue::from(child)))
            .collect(),
    )
}

// GuardrailValue's own Deserialize turns a non-integer number into the private-key object under arbitrary_precision
pub fn parse_request_data(source: &str) -> Result<RequestData, PassthroughPayloadError> {
    let wire = serde_json::from_str::<BTreeMap<String, WireValue>>(source)
        .map_err(PassthroughPayloadError::Parse)?;
    Ok(wire
        .into_iter()
        .map(|(key, value)| (key, GuardrailValue::from(value)))
        .collect())
}
