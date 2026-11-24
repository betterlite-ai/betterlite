use std::error::Error;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::descriptors::BatchLimitDescriptor;
use crate::api::request_destination::RequestData;

// betterlite-rate-limit depends on this crate, so its builders arrive through the constructor port
pub trait BatchLimiterSource {
    type Error: Error + Send + Sync + 'static;

    const PROJECT_OTPM_DESCRIPTOR_KEY: &'static str;

    fn descriptors_for(
        &self,
        auth: &UserAPIKeyAuth,
        data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, Self::Error>;

    fn project_io_for(
        &self,
        auth: &UserAPIKeyAuth,
        model: &str,
        out: &mut Vec<BatchLimitDescriptor>,
    );

    fn candidate_count(&self, body: &RequestData) -> i64;

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64;
}
