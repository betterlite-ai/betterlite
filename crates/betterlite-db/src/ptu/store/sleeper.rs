use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

// The retry delay is waited through this trait so a test records the schedule instead of sleeping it.
pub trait Sleeper {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        sleep(duration)
    }
}
