use crate::fallback_plans::groups::RequestMetadata;
use crate::fallback_plans::plan::RequestOverrides;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HopRequest<'request> {
    pub model: &'request str,
    pub overrides: RequestOverrides<'request>,
    // the attempt's own copy, so a write during the hop never reaches the outer request
    pub metadata: RequestMetadata,
}
