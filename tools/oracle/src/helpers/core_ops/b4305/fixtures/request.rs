use betterlite_cache::handler::retrieve::AsyncSupport;
use betterlite_core::caching::lookup::request::CacheLookup;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use chrono::DateTime;

use super::handlers::FIXED_TIME;
use super::input::{Operation, Scenario};

pub(crate) fn lookup_request(scenario: &Scenario) -> Result<CacheLookup, Box<HelperError>> {
    Ok(CacheLookup {
        model: scenario.model.clone(),
        call_type: scenario.call_type,
        start_time: DateTime::parse_from_rfc3339(FIXED_TIME).or_value_error()?,
        kwargs: scenario.kwargs.clone(),
        args: Vec::new(),
        converted_stream_requested: false,
        agentic_loop_depth: 0,
    })
}

// The S3 cache excludes partial embedding writes, and the sync path always reads the backend directly
pub(crate) const fn read_support(operation: Operation) -> AsyncSupport {
    match operation {
        Operation::Async => AsyncSupport::Supported,
        Operation::Sync => AsyncSupport::Unsupported,
    }
}
