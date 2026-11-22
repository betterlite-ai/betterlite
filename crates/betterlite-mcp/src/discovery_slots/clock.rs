use tokio::time::Instant;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioClock;

impl Clock for TokioClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
