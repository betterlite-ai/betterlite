use std::future::Future;
use std::io::Error as IoError;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::runtime::{Builder, Handle};
use tokio::task::block_in_place;

// The handler is synchronous: a blocking task or a plain thread drives the assembler, never an async worker.
pub(super) fn block_on<F>(future: F) -> Result<F::Output, IoError>
where
    F: Future,
{
    match Handle::try_current() {
        Ok(handle) => Ok(block_in_place(|| handle.block_on(future))),
        Err(_outside) => {
            let runtime = Builder::new_current_thread().enable_all().build()?;
            Ok(runtime.block_on(future))
        }
    }
}

// The response's created stamp and the chunk creation time, as LiteLLM's datetime.now() fills them.
pub(super) fn unix_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    i64::try_from(seconds).unwrap_or(i64::MAX)
}
