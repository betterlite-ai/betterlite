use std::error::Error;

use super::service_event::ServiceEvent;

pub trait ServiceHooks: Send + Sync {
    fn success(&self, event: &ServiceEvent);
    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync));
}
