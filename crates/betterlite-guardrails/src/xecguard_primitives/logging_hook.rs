use std::fmt::Display;

use tokio::runtime::{Builder, Handle};

#[must_use]
pub fn logging_hook<K, R, Hook, E>(mut kwargs: K, mut result: R, hook: Hook) -> (K, R)
where
    Hook: AsyncFnOnce(&mut K, &mut R) -> Result<(), E>,
    E: Display,
{
    if Handle::try_current().is_ok() {
        return (kwargs, result);
    }
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "XecGuard sync logging_hook swallowed exception"
            );
            return (kwargs, result);
        }
    };
    if let Err(error) = runtime.block_on(hook(&mut kwargs, &mut result)) {
        tracing::debug!(
            error = %error,
            "XecGuard sync logging_hook swallowed exception"
        );
    }
    (kwargs, result)
}

mod tests;
