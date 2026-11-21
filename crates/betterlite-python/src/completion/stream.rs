use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
mod tests;
// Every chunk is already buffered by the SDK's request driver, so the stream only hands them out in order.
struct StreamFeed {
    chunks: VecDeque<String>,
    closed: bool,
}
impl StreamFeed {
    fn take_next(&mut self) -> Option<String> {
        if self.closed {
            return None;
        }
        self.chunks.pop_front()
    }
    fn close(&mut self) {
        self.closed = true;
        self.chunks.clear();
    }
}
// The awaitable chunk iterator litellm's streaming responses expose from Python.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeChatStream {
    feed: Arc<Mutex<StreamFeed>>,
}
impl From<Vec<String>> for NativeChatStream {
    fn from(chunks: Vec<String>) -> Self {
        Self {
            feed: Arc::new(Mutex::new(StreamFeed {
                chunks: VecDeque::from(chunks),
                closed: false,
            })),
        }
    }
}
#[pymethods]
impl NativeChatStream {
    // the blocking form the synchronous completion returns, running the lock wait with the GIL released
    fn next_chunk(&self, py: Python<'_>) -> Option<String> {
        let feed = Arc::clone(&self.feed);
        py.detach(move || {
            // the GIL is released here, so the caller's other threads run while this one waits on the lock
            get_runtime().block_on(async move { feed.lock().await.take_next() })
        })
    }
    fn anext_chunk<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let feed = Arc::clone(&self.feed);
        let next = async move { Ok(feed.lock().await.take_next()) };
        future_into_py(py, next)
    }
    fn close(&self, py: Python<'_>) {
        let feed = Arc::clone(&self.feed);
        py.detach(move || {
            get_runtime().block_on(async move { feed.lock().await.close() });
        });
    }
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.close(py);
        future_into_py(py, async { Ok::<(), PyErr>(()) })
    }
}
