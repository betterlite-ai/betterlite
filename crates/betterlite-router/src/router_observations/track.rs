use super::RouterObservations;
use super::keys::ROUTER_COUNTER_TTL;

impl RouterObservations<'_> {
    #[must_use]
    pub fn update_usage(&self, deployment_id: &str) -> i64 {
        let Some(current) = self.counters.read(deployment_id) else {
            self.counters
                .write(deployment_id, 1, Some(ROUTER_COUNTER_TTL));
            return 1;
        };
        let request_count = current.saturating_add(1);
        // the later writes keep the ttl the first one stored
        self.counters.write(deployment_id, request_count, None);
        request_count
    }

    pub fn track_deployment_metrics(&self, model_id: Option<&str>, response_present: bool) {
        if response_present {
            return;
        }
        if let Some(model_id) = model_id {
            let _request_count = self.update_usage(model_id);
        }
    }
}
