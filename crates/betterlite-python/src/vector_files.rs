mod calls;
mod convert;
mod error;
mod native;
mod service;
mod session;
mod source;
mod write_calls;
mod write_service;

use pyo3::prelude::*;

use self::native::NativeVectorFileSession;

// The vector store file session and the exception every failed call of it carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    error::register(module)?;
    module.add_class::<NativeVectorFileSession>()?;
    Ok(())
}
