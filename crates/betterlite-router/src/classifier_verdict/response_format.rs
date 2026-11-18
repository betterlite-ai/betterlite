use serde::Serialize;

pub const TIER_CLASSIFICATION_DESCRIPTION: &str =
    "Structured response schema for the LLM-based complexity classifier.";

// declaration order is the packaged wrapper's order, so the serialized bytes match LiteLLM's
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TierClassificationFormat {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub json_schema: Box<JsonSchemaFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JsonSchemaFormat {
    pub schema: Box<ClassificationSchema>,
    pub name: &'static str,
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassificationSchema {
    pub description: &'static str,
    pub properties: Box<TierProperties>,
    pub required: [&'static str; 1],
    pub title: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(rename = "additionalProperties")]
    pub additional_properties: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TierProperties {
    pub tier: Box<TierSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TierSchema {
    #[serde(rename = "enum")]
    pub choices: Vec<String>,
    pub title: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
}

#[must_use]
pub fn tier_classification_format(labels: &[&str]) -> TierClassificationFormat {
    TierClassificationFormat {
        kind: "json_schema",
        json_schema: Box::new(JsonSchemaFormat {
            schema: Box::new(ClassificationSchema {
                description: TIER_CLASSIFICATION_DESCRIPTION,
                properties: Box::new(TierProperties {
                    tier: Box::new(TierSchema {
                        choices: labels.iter().map(|label| (*label).to_owned()).collect(),
                        title: "Tier",
                        kind: "string",
                    }),
                }),
                required: ["tier"],
                title: "TierClassification",
                kind: "object",
                additional_properties: false,
            }),
            name: "TierClassification",
            strict: true,
        }),
    }
}
