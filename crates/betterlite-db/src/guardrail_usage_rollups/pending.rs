use indexmap::IndexMap;

use super::{GuardrailCounts, MetricsKey, UsageUnitIncrement, UsageUnitKey};

// Rows whose connection-error retries ran out, held until the next flush; each type caps its own queue
#[derive(Debug, Default)]
pub struct PendingRollups {
    pub metrics: IndexMap<MetricsKey, GuardrailCounts>,
    pub units: IndexMap<UsageUnitKey, UsageUnitIncrement>,
}
