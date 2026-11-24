mod compatible;
mod convert;
mod error;
mod handlers;
mod lookup;
mod native;
mod settings;
mod snapshot;
mod tests;

use pyo3::prelude::*;

pub(crate) use native::NativeProviders;

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeProviders>()?;
    Ok(())
}
