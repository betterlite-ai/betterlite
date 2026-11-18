use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

// The six classic invoke families LiteLLM keeps as separate config classes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Ai21,
    Cohere,
    Llama,
    Mistral,
    Titan,
    Anthropic,
}

// The class dictionary entries get_config filters out, plus the two non-JSON values it keeps.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeKind {
    Function,
    Builtin,
    ClassMethod,
    StaticMethod,
    Descriptor,
    AbcData,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ClassAttribute {
    Special { kind: AttributeKind },
    Value(Json),
}

pub type ClassAttributes = IndexMap<String, ClassAttribute>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyConfig {
    pub family: Family,
    pub attributes: ClassAttributes,
}

impl LegacyConfig {
    // The class dictionary a fresh Python class carries: its _abc_impl marker and Claude2's import-time limit.
    #[must_use]
    pub fn new(family: Family, default_max_tokens: i64) -> Self {
        let mut attributes = ClassAttributes::new();
        attributes.insert(
            "_abc_impl".to_owned(),
            ClassAttribute::Special {
                kind: AttributeKind::AbcData,
            },
        );
        if matches!(family, Family::Anthropic) {
            attributes.insert(
                "max_tokens_to_sample".to_owned(),
                ClassAttribute::Value(Json::Number(Number::from(default_max_tokens))),
            );
        }
        Self { family, attributes }
    }

    #[must_use]
    pub fn get_config(&self) -> ClassAttributes {
        self.attributes
            .iter()
            .filter(|(name, attribute)| visible(self.family, name, attribute))
            .map(|(name, attribute)| (name.clone(), attribute.clone()))
            .collect()
    }
}

fn visible(family: Family, name: &str, attribute: &ClassAttribute) -> bool {
    if name.starts_with("__") {
        return false;
    }
    // Claude2's get_config has no _abc filter; the other five classes drop those names.
    if !matches!(family, Family::Anthropic) && name.starts_with("_abc") {
        return false;
    }
    match attribute {
        ClassAttribute::Value(value) => !matches!(value, Json::Null),
        ClassAttribute::Special { kind } => {
            matches!(kind, AttributeKind::Descriptor | AttributeKind::AbcData)
        }
    }
}
