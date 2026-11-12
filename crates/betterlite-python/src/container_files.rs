use pyo3::prelude::*;

pub const fn register(_module: &Bound<'_, PyModule>) -> PyResult<()> {
    // The container_files family task adds its native functions here.
    Ok(())
}
