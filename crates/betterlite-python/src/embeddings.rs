mod call;
mod convert;
mod error;
mod native;
mod request;
mod response;
mod session;
use self::error::NativeEmbeddingError;
use self::native::NativeEmbeddingSession;
use pyo3::prelude::*;
// main.py's embedding entry points: one session, its two call methods and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeEmbeddingSession>()?;
    module.add(
        "NativeEmbeddingError",
        module.py().get_type::<NativeEmbeddingError>(),
    )?;
    Ok(())
}
