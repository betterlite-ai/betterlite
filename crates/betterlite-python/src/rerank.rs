use self::error::NativeRerankError;
use self::native::NativeRerankSession;
use pyo3::prelude::*;

mod call;
mod convert;
mod error;
mod native;
mod operations;
mod options;
mod session;

// main.py's rerank entry points: one session, its two call methods and the exception every failure carries.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeRerankSession>()?;
    module.add(
        "NativeRerankError",
        module.py().get_type::<NativeRerankError>(),
    )?;
    Ok(())
}
