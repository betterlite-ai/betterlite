use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use tokio::runtime::{Handle, Runtime};

// Wraps time.time(), datetime.datetime.now() and the monotonic clock message copying reads
pub trait WrapperClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
    fn monotonic(&self) -> Duration;
}

// uuid.uuid4() for litellm_call_id and the trace id function_setup asks for
pub trait IdSource: Send + Sync {
    fn next_call_id(&self) -> String;
}

// call_blocking drives the async target and the async ports from a caller that is not in a runtime
pub trait BlockingBridge {
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future;
}

impl BlockingBridge for Runtime {
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        Self::block_on(self, future)
    }
}

impl BlockingBridge for Handle {
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        Self::block_on(self, future)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallTimes {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}
