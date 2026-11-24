mod convert;
mod engine;
mod error;
mod native;

use pyo3::prelude::*;

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<native::NativeCostExtra>()?;
    Ok(())
}
