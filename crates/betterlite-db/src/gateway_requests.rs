use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use self::gateway_request_counts::GatewayRequestCounts;

pub mod gateway_request_counts;
mod tests;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GatewayRequestKey {
    pub date: String,
    pub category: String,
    pub route: String,
}

pub type GatewayRequestSnapshot = BTreeMap<GatewayRequestKey, GatewayRequestCounts>;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayRequestBreakdownEntry {
    pub category: String,
    pub route: String,
    #[serde(default)]
    pub successful_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayRequestDailyEntry {
    pub date: String,
    #[serde(default)]
    pub successful_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GatewayRequestActivityResponse {
    #[serde(default)]
    pub total_successful_requests: i64,
    #[serde(default)]
    pub total_failed_requests: i64,
    #[serde(default)]
    pub by_date: Vec<GatewayRequestDailyEntry>,
    #[serde(default)]
    pub by_route: Vec<GatewayRequestBreakdownEntry>,
}
