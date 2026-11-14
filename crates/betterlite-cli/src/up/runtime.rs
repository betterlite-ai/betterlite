use std::future::Future;
use std::io;

use tokio::runtime::Builder;

pub(super) fn block_on<F>(future: F) -> Result<F::Output, io::Error>
where
    F: Future,
{
    let runtime = Builder::new_current_thread().build()?;
    let output = runtime.block_on(future);
    Ok(output)
}
