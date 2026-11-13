use std::sync::{Arc, Mutex, PoisonError};

#[derive(Default)]
pub(super) struct ClockState {
    pub now: f64,
    pub fails: bool,
}

pub(super) type SharedClock = Arc<Mutex<ClockState>>;

pub(super) fn switchable() -> (SharedClock, Arc<dyn Fn() -> f64 + Send + Sync>) {
    let state: SharedClock = Arc::new(Mutex::new(ClockState::default()));
    let clock_state = Arc::clone(&state);
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(move || {
        let locked = clock_state.lock().unwrap_or_else(PoisonError::into_inner);
        if locked.fails { f64::NAN } else { locked.now }
    });
    (state, clock)
}
