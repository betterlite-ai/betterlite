mod tests;

use std::time::Duration;

use tokio::time::Instant;

use super::State;

impl State {
    pub(super) fn consume_resync_budget(&mut self, max_resyncs: usize, window: Duration) -> bool {
        let now = Instant::now();
        let window_ended = self
            .window_started_at
            .is_none_or(|started_at| now.saturating_duration_since(started_at) >= window);
        if window_ended {
            self.window_started_at = Some(now);
            self.window_resyncs = 0;
        }
        if self.window_resyncs >= max_resyncs {
            return false;
        }
        self.window_resyncs = self.window_resyncs.saturating_add(1);
        true
    }
}
