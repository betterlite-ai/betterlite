use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;

pub mod deployment;
pub mod interval;
pub mod resolver;
mod seconds;
pub mod wrapper;

pub type JsonObject = BTreeMap<String, JsonValue>;

pub const KEEPALIVE_MIN_SECONDS: f64 = 1.0;
pub const KEEPALIVE_MAX_SECONDS: f64 = 300.0;
pub const KEEPALIVE_CACHE_TTL: Duration = Duration::from_secs(5);

pub trait KeepaliveRouter: Send + Sync {
    fn deployment_params(&self, model_id: &str) -> Option<JsonObject>;

    fn group_deployments(&self, model_name: Option<&str>) -> Vec<JsonValue>;
}

mod tests;
