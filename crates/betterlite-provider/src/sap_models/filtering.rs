use super::presence::Field;
use super::presence::optional;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

use super::SapModelError;
use super::presence::{Given, NonEmpty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize_repr, Serialize_repr)]
#[repr(u8)]
pub enum AzureThreshold {
    AllowSafe = 0,
    AllowSafeLow = 2,
    AllowSafeLowMedium = 4,
    AllowAll = 6,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureContentFilter {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub hate: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub sexual: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub violence: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub self_harm: Field<AzureThreshold>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureContentSafetyInput {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub hate: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub sexual: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub violence: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub self_harm: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub prompt_shield: Field<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureContentSafetyOutput {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub hate: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub sexual: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub violence: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub self_harm: Field<AzureThreshold>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub protected_material_code: Field<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlamaGuard38bFilter {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub violent_crimes: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub non_violent_crimes: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub sex_crimes: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub child_exploitation: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub defamation: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub specialized_advice: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub privacy: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub intellectual_property: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub indiscriminate_weapons: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub hate: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub self_harm: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub sexual_content: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub elections: Given<bool>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub code_interpreter_abuse: Given<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum LlamaGuardKind {
    #[serde(rename = "llama_guard_3_8b")]
    LlamaGuard38b,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum AzureKind {
    #[serde(rename = "azure_content_safety")]
    AzureContentSafety,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlamaGuard38bFilterConfig {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<LlamaGuardKind>,
    pub config: LlamaGuard38bFilter,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureInputFilterConfig {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<AzureKind>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub config: Field<AzureContentSafetyInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureOutputFilterConfig {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<AzureKind>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub config: Field<AzureContentSafetyOutput>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(try_from = "i64")]
pub struct Overlap(i64);

impl TryFrom<i64> for Overlap {
    type Error = SapModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if (0..=10_000).contains(&value) {
            Ok(Self(value))
        } else {
            Err(SapModelError::Rejected {
                message: String::from("overlap must be between 0 and 10000"),
            })
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilteringStreamOptions {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub overlap: Field<Overlap>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InputFilterConfig {
    Azure(AzureInputFilterConfig),
    LlamaGuard(LlamaGuard38bFilterConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OutputFilterConfig {
    Azure(AzureOutputFilterConfig),
    LlamaGuard(LlamaGuard38bFilterConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InputFiltering {
    pub filters: NonEmpty<InputFilterConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputFiltering {
    pub filters: NonEmpty<OutputFilterConfig>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub stream_options: Field<FilteringStreamOptions>,
}

#[derive(Debug, Deserialize)]
struct FilteringModuleConfigRaw {
    #[serde(default, with = "optional")]
    input: Field<InputFiltering>,
    #[serde(default, with = "optional")]
    output: Field<OutputFiltering>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "FilteringModuleConfigRaw")]
pub struct FilteringModuleConfig {
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub input: Field<InputFiltering>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub output: Field<OutputFiltering>,
}

impl TryFrom<FilteringModuleConfigRaw> for FilteringModuleConfig {
    type Error = SapModelError;

    fn try_from(raw: FilteringModuleConfigRaw) -> Result<Self, Self::Error> {
        let has_input = matches!(&raw.input, Some(Some(_)));
        let has_output = matches!(&raw.output, Some(Some(_)));
        if !has_input && !has_output {
            return Err(SapModelError::Rejected {
                message: String::from(
                    "For using SAP Filtering Module you must provide at least one property: input or output filters.",
                ),
            });
        }
        Ok(Self {
            input: raw.input,
            output: raw.output,
        })
    }
}
