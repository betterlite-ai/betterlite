mod convert;
mod error;
mod native;
mod seams;
mod session;
mod source;

use pyo3::prelude::*;

use self::native::NativeVectorSearchSession;

// The vector store create/search session and the exception every failed call of it carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    error::register(module)?;
    module.add_class::<NativeVectorSearchSession>()?;
    Ok(())
}
