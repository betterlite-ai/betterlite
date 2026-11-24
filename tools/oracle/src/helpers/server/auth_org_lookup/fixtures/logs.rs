use std::sync::{Arc, PoisonError};

use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::fakes::Script;

// the levels the owned module logged during the step, in order
pub(crate) struct Levels {
    pub(crate) script: Arc<Script>,
}

impl<S: Subscriber> Layer<S> for Levels {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata
            .target()
            .starts_with("betterlite_auth::auth_org_lookup")
        {
            self.script
                .board
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .logs
                .push(metadata.level().to_string());
        }
    }
}
