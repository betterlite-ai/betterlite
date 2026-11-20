use std::sync::Arc;

use betterlite_db::gateway_request_tracking::GatewayRequestClock;

use super::store::UserAgentAnalyticsStore;

#[derive(Clone)]
pub struct UserAgentAnalyticsState {
    pub store: Option<Arc<dyn UserAgentAnalyticsStore>>,
    pub clock: Arc<dyn GatewayRequestClock>,
}
