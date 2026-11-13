use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use tokio::time::Instant;

use super::interval::resolve_keepalive_seconds;
use super::{JsonObject, KEEPALIVE_CACHE_TTL, KeepaliveRouter};
use crate::keepalive_interval::PingSetting;

pub struct KeepaliveResolver {
    router: Option<Arc<dyn KeepaliveRouter>>,
    request_data: Arc<JsonObject>,
    global: Option<PingSetting>,
    last_model_id: Option<String>,
    last_value: f64,
    last_resolved_at: Instant,
}

impl KeepaliveResolver {
    #[must_use]
    pub fn new(
        router: Option<Arc<dyn KeepaliveRouter>>,
        request_data: Arc<JsonObject>,
        global: Option<PingSetting>,
    ) -> Self {
        Self {
            router,
            request_data,
            global,
            last_model_id: None,
            last_value: 0.0,
            last_resolved_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn resolve(&mut self, hidden: &JsonObject) -> f64 {
        let model_key = hidden
            .get("model_id")
            .and_then(JsonValue::as_str)
            .filter(|model_id| !model_id.is_empty());
        let now = Instant::now();
        if let Some(model_id) = model_key {
            let within_ttl =
                now.saturating_duration_since(self.last_resolved_at) < KEEPALIVE_CACHE_TTL;
            if within_ttl && self.last_model_id.as_deref() == Some(model_id) {
                return self.last_value;
            }
        }
        let value = resolve_keepalive_seconds(
            self.router.as_deref(),
            &self.request_data,
            hidden,
            self.global.as_ref(),
        );
        if let Some(model_id) = model_key {
            self.last_model_id = Some(model_id.to_owned());
            self.last_value = value;
            self.last_resolved_at = now;
        }
        value
    }
}
