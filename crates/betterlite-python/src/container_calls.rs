mod convert;
mod error;
mod session;
mod stack;
mod transport;

use pyo3::prelude::*;
use pyo3::types::PyType;

use self::session::NativeContainerSession;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let base = module.getattr("NativeError")?;
    let base_type = base.cast::<PyType>()?;
    let error_type = PyErr::new_type(
        module.py(),
        c"betterlite._native.NativeContainerError",
        None,
        Some(base_type),
        None,
    )?;
    module.add("NativeContainerError", error_type)?;
    module.add_class::<NativeContainerSession>()?;
    Ok(())
}
