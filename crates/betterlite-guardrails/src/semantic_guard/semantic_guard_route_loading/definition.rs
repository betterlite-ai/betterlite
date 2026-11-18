use std::collections::BTreeMap;
use std::sync::OnceLock;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use regex::Regex;
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

// The implicit resolvers of PyYAML 6.0.2's yaml/resolver.py for bool, float, int and null, in one
// pattern with the re.X whitespace kept for reading; a plain scalar matching one is not text.
const PYYAML_NON_TEXT_SCALAR: &str = r"(?x)^(?:
    yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF
  | [-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?
  | \.[0-9][0-9_]*(?:[eE][-+][0-9]+)?
  | [-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*
  | [-+]?\.(?:inf|Inf|INF)
  | \.(?:nan|NaN|NAN)
  | [-+]?0b[0-1_]+
  | [-+]?0[0-7_]+
  | [-+]?(?:0|[1-9][0-9_]*)
  | [-+]?0x[0-9a-fA-F_]+
  | [-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+
  | ~|null|Null|NULL|
)$";

// PyYAML reads a plain scalar as text unless one of those resolvers matches. A timestamp needs no
// resolver here, because serde already reads one as text and the typed parse never calls this.
fn pyyaml_resolves_scalar_non_text(text: &str) -> bool {
    static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(PYYAML_NON_TEXT_SCALAR).ok());
    pattern
        .as_ref()
        .is_none_or(|pattern| pattern.is_match(text))
}
