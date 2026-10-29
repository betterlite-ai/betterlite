use serde::{Deserialize, Serialize};

pub mod cache;
pub mod input;
pub mod other;
pub mod output;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PricingValues {
    #[serde(flatten)]
    pub cache: cache::CachePricing,
    #[serde(flatten)]
    pub input: input::InputPricing,
    #[serde(flatten)]
    pub output: output::OutputPricing,
    #[serde(flatten)]
    pub other: other::OtherPricing,
}
