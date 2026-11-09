use super::presence::Field;
use super::presence::optional;
use serde::{Deserialize, Serialize};

use super::SapModelError;
use super::presence::Given;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorCategory {
    Placeholders,
    TemplateRoles,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranslationSelector {
    pub category: SelectorCategory,
    pub items: Vec<String>,
    pub source_language: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InputTranslationConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub source_language: Field<String>,
    pub target_language: String,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub apply_to: Field<Vec<TranslationSelector>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TargetLanguage {
    Text(String),
    Selector(TranslationSelector),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputTranslationConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub source_language: Field<String>,
    pub target_language: TargetLanguage,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationKind {
    SapDocumentTranslation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPDocumentTranslationInput {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<TranslationKind>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub translate_messages_history: Field<bool>,
    pub config: InputTranslationConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPDocumentTranslationOutput {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<TranslationKind>,
    pub config: OutputTranslationConfig,
}

#[derive(Debug, Deserialize)]
struct TranslationModuleConfigRaw {
    #[serde(default, with = "optional")]
    input: Field<SAPDocumentTranslationInput>,
    #[serde(default, with = "optional")]
    output: Field<SAPDocumentTranslationOutput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "TranslationModuleConfigRaw")]
pub struct TranslationModuleConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub input: Field<SAPDocumentTranslationInput>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub output: Field<SAPDocumentTranslationOutput>,
}

impl TryFrom<TranslationModuleConfigRaw> for TranslationModuleConfig {
    type Error = SapModelError;

    fn try_from(raw: TranslationModuleConfigRaw) -> Result<Self, Self::Error> {
        if matches!(&raw.input, None | Some(None)) && matches!(&raw.output, None | Some(None)) {
            return Err(SapModelError::Rejected {
                message: String::from(
                    "TranslationModuleConfig requires at least one of 'input' or 'output'.",
                ),
            });
        }
        Ok(Self {
            input: raw.input,
            output: raw.output,
        })
    }
}
