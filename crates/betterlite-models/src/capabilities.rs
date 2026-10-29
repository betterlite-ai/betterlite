use serde::{Deserialize, Serialize};

mod flag;
pub mod media;
pub mod reasoning;
pub mod tooling;
pub mod vendor;

pub(crate) use flag::capability_struct;

/// Every `supports_*` style boolean the pinned catalog carries, grouped by
/// what they gate. A flag is `Option<bool>` because an absent flag is not
/// the same claim as an explicit `false` (see pitfalls).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityFields {
    #[serde(flatten)]
    pub reasoning: reasoning::ReasoningCapabilities,
    #[serde(flatten)]
    pub media: media::MediaCapabilities,
    #[serde(flatten)]
    pub tooling: tooling::ToolingCapabilities,
    #[serde(flatten)]
    pub vendor: vendor::VendorCapabilities,
}

mod tests;
