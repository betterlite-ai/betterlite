mod convert;
mod dispatch;
mod error;
mod session;

use self::error::NativeFileCallError;
use pyo3::prelude::*;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<session::NativeFileCallSession>()?;
    module.add(
        "NativeFileCallError",
        module.py().get_type::<NativeFileCallError>(),
    )?;
    Ok(())
}
