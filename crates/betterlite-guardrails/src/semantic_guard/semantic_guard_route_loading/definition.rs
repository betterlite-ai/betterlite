use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde::de::IgnoredAny;

mod from_value;

#[derive(Debug, Clone, PartialEq)]
pub struct RouteDefinition {
    pub route_name: String,
    pub description: String,
    pub utterances: Vec<String>,
    pub similarity_threshold: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum DefinitionError {
    #[error("the route definition is not a mapping")]
    NotAMapping,
    #[error("the route definition is missing the required field `{field}`")]
    MissingField { field: &'static str },
    #[error("the route definition field `{field}` must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the route definition has an unknown field `{field}`")]
    UnknownField { field: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDefinition {
    route_name: Option<GuardrailValue>,
    description: Option<GuardrailValue>,
    utterances: Option<GuardrailValue>,
    similarity_threshold: Option<GuardrailValue>,
    // the three Route fields a loaded route never fills stay readable, as LiteLLM drops them from its route definitions
    #[serde(rename = "function_schemas")]
    _function_schemas: Option<IgnoredAny>,
    #[serde(rename = "llm")]
    _llm: Option<IgnoredAny>,
    #[serde(rename = "metadata")]
    _metadata: Option<IgnoredAny>,
}

// The same definition with its text fields read as raw scalars: the typed parse above already
// proves every field's shape, so this one only says what PyYAML read each text scalar as.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDefinitionText {
    route_name: Option<String>,
    description: Option<String>,
    utterances: Option<Vec<String>>,
    #[serde(rename = "similarity_threshold")]
    _similarity_threshold: Option<IgnoredAny>,
    #[serde(rename = "function_schemas")]
    _function_schemas: Option<IgnoredAny>,
    #[serde(rename = "llm")]
    _llm: Option<IgnoredAny>,
    #[serde(rename = "metadata")]
    _metadata: Option<IgnoredAny>,
}

impl RawDefinition {
    // LiteLLM reads a route file with PyYAML, which leaves a plain scalar such as 1e3 or y a
    // string where serde's inference sees a number or a boolean; the caller passes the same
    // document read as text so those scalars keep PyYAML's value.
    #[must_use]
    pub(super) fn with_pyyaml_scalars(mut self, text: Option<&RawDefinitionText>) -> Self {
        let Some(text) = text else {
            return self;
        };
        reclassify(&mut self.route_name, text.route_name.as_deref());
        reclassify(&mut self.description, text.description.as_deref());
        let (Some(GuardrailValue::Array(utterances)), Some(texts)) =
            (self.utterances.as_mut(), text.utterances.as_ref())
        else {
            return self;
        };
        if utterances.len() == texts.len() {
            for (utterance, raw) in utterances.iter_mut().zip(texts) {
                reclassify_scalar(utterance, raw);
            }
        }
        self
    }

    pub(super) fn into_definition(self) -> Result<RouteDefinition, DefinitionError> {
        let mut fields = BTreeMap::new();
        let known = [
            ("route_name", self.route_name),
            ("description", self.description),
            ("utterances", self.utterances),
            ("similarity_threshold", self.similarity_threshold),
        ];
        for (name, value) in known {
            if let Some(present) = value {
                fields.insert(name.to_owned(), present);
            }
        }
        // the file route and the inline route go through one validator, so a wrong type names the same field
        RouteDefinition::from_value(&GuardrailValue::Object(fields))
    }
}

fn reclassify(field: &mut Option<GuardrailValue>, text: Option<&str>) {
    let (Some(value), Some(text)) = (field.as_mut(), text) else {
        return;
    };
    reclassify_scalar(value, text);
}

fn reclassify_scalar(value: &mut GuardrailValue, text: &str) {
    let inferred_non_text = matches!(
        value,
        GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
    );
    if inferred_non_text && !pyyaml_resolves_scalar_non_text(text) {
        *value = GuardrailValue::Text(text.to_owned());
    }
}

// PyYAML reads a plain scalar as text unless its boolean, integer or float resolver matches; null
// and timestamps already arrive from the typed parse as a null or a text, so they never get here.
fn pyyaml_resolves_scalar_non_text(text: &str) -> bool {
    if pyyaml_boolean(text) {
        return true;
    }
    pyyaml_integer(text) || pyyaml_float(text)
}

// PyYAML's bool resolver, whose forms are the only spellings it reads as a boolean.
fn pyyaml_boolean(text: &str) -> bool {
    matches!(
        text,
        "yes"
            | "Yes"
            | "YES"
            | "no"
            | "No"
            | "NO"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | "on"
            | "On"
            | "ON"
            | "off"
            | "Off"
            | "OFF"
    )
}

fn pyyaml_integer(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    pyyaml_binary_integer(unsigned)
        || pyyaml_octal_integer(unsigned)
        || unsigned == "0"
        || pyyaml_positive_decimal(unsigned)
        || pyyaml_hex_integer(unsigned)
        || pyyaml_sexagesimal_integer(unsigned)
}

fn pyyaml_binary_integer(unsigned: &str) -> bool {
    unsigned.strip_prefix("0b").is_some_and(|digits| {
        !digits.is_empty()
            && digits
                .bytes()
                .all(|byte| matches!(byte, b'0' | b'1' | b'_'))
    })
}

fn pyyaml_octal_integer(unsigned: &str) -> bool {
    unsigned.len() > 1
        && unsigned.starts_with('0')
        && unsigned
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'7' | b'_'))
}

fn pyyaml_positive_decimal(digits: &str) -> bool {
    let Some(first) = digits.as_bytes().first() else {
        return false;
    };
    (b'1'..=b'9').contains(first)
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
}

fn pyyaml_hex_integer(unsigned: &str) -> bool {
    unsigned.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty()
            && digits
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'_')
    })
}

fn pyyaml_sexagesimal_integer(unsigned: &str) -> bool {
    let Some((head, rest)) = unsigned.split_once(':') else {
        return false;
    };
    pyyaml_positive_decimal(head) && rest.split(':').all(pyyaml_sexagesimal_part)
}

fn pyyaml_sexagesimal_part(part: &str) -> bool {
    let mut bytes = part.bytes();
    let Some(tens) = bytes.next() else {
        return false;
    };
    if !tens.is_ascii_digit() {
        return false;
    }
    let Some(units) = bytes.next() else {
        return true;
    };
    tens <= b'5' && units.is_ascii_digit() && bytes.next().is_none()
}

fn pyyaml_float(text: &str) -> bool {
    matches!(text, ".nan" | ".NaN" | ".NAN")
        || pyyaml_decimal_float(text)
        || pyyaml_sexagesimal_float(text)
        || matches!(
            text.strip_prefix(['+', '-']).unwrap_or(text),
            ".inf" | ".Inf" | ".INF"
        )
}

fn pyyaml_decimal_float(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let Some((mantissa, exponent)) = unsigned.split_once(['e', 'E']) else {
        return pyyaml_float_mantissa(unsigned);
    };
    pyyaml_signed_exponent(exponent) && pyyaml_float_mantissa(mantissa)
}

fn pyyaml_signed_exponent(exponent: &str) -> bool {
    let Some(digits) = exponent.strip_prefix(['+', '-']) else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn pyyaml_float_mantissa(mantissa: &str) -> bool {
    let Some((whole, fraction)) = mantissa.split_once('.') else {
        return false;
    };
    if whole.is_empty() {
        return pyyaml_digits(fraction);
    }
    pyyaml_digits(whole)
        && fraction
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
}

fn pyyaml_sexagesimal_float(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let Some((whole, fraction)) = unsigned.rsplit_once('.') else {
        return false;
    };
    let Some((head, rest)) = whole.split_once(':') else {
        return false;
    };
    pyyaml_digits(head)
        && rest.split(':').all(pyyaml_sexagesimal_part)
        && fraction
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
}

fn pyyaml_digits(value: &str) -> bool {
    let Some(first) = value.as_bytes().first() else {
        return false;
    };
    first.is_ascii_digit()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
}
