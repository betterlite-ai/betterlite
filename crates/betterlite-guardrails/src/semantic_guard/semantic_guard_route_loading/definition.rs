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

impl RawDefinition {
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
