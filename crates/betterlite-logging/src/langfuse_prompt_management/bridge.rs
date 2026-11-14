use std::future::Future;
use std::io::Error as IoError;
use std::thread;

use thiserror::Error;
use tokio::runtime::{Builder, Handle};

// asyncify.run_async_function: a sync callback may drive an async one without nesting runtimes
#[derive(Debug, Error)]
pub(crate) enum BridgeError {
    #[error("the async callback thread stopped without a result")]
    Panicked,
    #[error("the async callback runtime could not be built")]
    Runtime {
        #[source]
        source: IoError,
    },
}

pub(crate) fn run_async_function<F, T>(future: F) -> Result<T, BridgeError>
where
    F: Future<Output = T> + Send,
    T: Send,
{
    if Handle::try_current().is_err() {
        return block_on(future);
    }
    // a runtime is already driving this thread, so the new one runs on a scoped thread of its own
    thread::scope(move |scope| {
        scope
            .spawn(move || block_on(future))
            .join()
            .unwrap_or_else(|_panicked| Err(BridgeError::Panicked))
    })
}

fn block_on<F, T>(future: F) -> Result<T, BridgeError>
where
    F: Future<Output = T>,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| BridgeError::Runtime { source })?;
    Ok(runtime.block_on(future))
}
