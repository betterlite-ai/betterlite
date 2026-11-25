mod convert;
mod dispatch;
mod error;
mod session;
mod stream;

use self::error::NativeFileContentError;
use pyo3::prelude::*;

// The session, the native stream and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<session::NativeFileContentSession>()?;
    module.add_class::<stream::NativeFileStream>()?;
    module.add(
        "NativeFileContentError",
        module.py().get_type::<NativeFileContentError>(),
    )?;
    Ok(())
}
