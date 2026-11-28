mod convert;
mod error;
mod native;
mod service;
mod session;
mod source;

use pyo3::prelude::*;

use self::native::NativeVectorManageSession;

// The vector store manage session and the exception every failed call of it carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    error::register(module)?;
    module.add_class::<NativeVectorManageSession>()?;
    Ok(())
}
