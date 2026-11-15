use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OutageRecord {
    pub model_id: String,
    pub alerts: Vec<i64>,
    pub minor_alert_sent: bool,
    pub major_alert_sent: bool,
    pub last_updated_at: f64,
}

// Python caches the deployment ids as list(set(...)), so the cached value is a list and reading it back restores the set.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RegionOutageRecord {
    pub provider_region_id: String,
    pub alerts: Vec<i64>,
    pub minor_alert_sent: bool,
    pub major_alert_sent: bool,
    pub last_updated_at: f64,
    pub deployment_ids: BTreeSet<String>,
}
