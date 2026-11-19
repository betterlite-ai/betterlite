use chrono::DateTime;
use chrono::Utc;

#[path = "gateway_request_tracking/tests/support.rs"]
pub(crate) mod test_support;

pub mod commit_gateway_requests;
pub mod flush_gateway_requests;
pub mod fold_gateway_counts;
pub mod gateway_request_accumulator;
pub mod gateway_request_redis;
pub mod gateway_requests_upsert;

pub const GATEWAY_REQUEST_DATE_FORMAT: &str = "%Y-%m-%d";

pub trait GatewayRequestClock: Send + Sync {
    fn utc_now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl GatewayRequestClock for SystemClock {
    fn utc_now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
