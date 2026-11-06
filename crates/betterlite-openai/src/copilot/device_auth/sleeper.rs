use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

pub trait Sleeper {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Default)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        sleep(duration)
    }
}
