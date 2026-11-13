use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::sleep as tokio_sleep;

pub trait Sleep: Send + Sync + Debug {
    fn sleep(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioSleep;

impl Sleep for TokioSleep {
    fn sleep(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio_sleep(delay))
    }
}
