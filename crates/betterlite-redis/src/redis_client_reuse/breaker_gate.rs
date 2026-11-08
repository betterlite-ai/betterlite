use std::error::Error;

use super::breaker::{BreakerAdmission, BreakerOpen};

pub trait BreakerGate: Send + Sync {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen>;
    fn exit(&self, admission: BreakerAdmission);
    fn record_failure(&self, error: &(dyn Error + Send + Sync));
    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync));
}
