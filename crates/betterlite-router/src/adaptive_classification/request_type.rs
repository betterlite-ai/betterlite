use serde::{Deserialize, Serialize};

#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RequestType {
    CodeGeneration,
    CodeUnderstanding,
    TechnicalDesign,
    AnalyticalReasoning,
    Writing,
    FactualLookup,
    General,
}
