use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use rust_decimal::Decimal;

pub type CounterReadFuture<'cache, Failure> =
    Pin<Box<dyn Future<Output = Result<Option<Decimal>, Failure>> + Send + 'cache>>;

pub type CounterWriteFuture<'cache, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'cache>>;

// DualCache's two levels as a window reset sees them: the in-memory set cannot fail, the Redis set can and is reported for a warning, and a read answers None when no number is cached.
pub trait WindowCounterCache: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    fn set_memory(&self, key: &str, value: Decimal);

    fn set_redis<'cache>(
        &'cache self,
        key: &'cache str,
        value: Decimal,
    ) -> CounterWriteFuture<'cache, Self::Failure>;

    fn read<'cache>(&'cache self, key: &'cache str) -> CounterReadFuture<'cache, Self::Failure>;
}
