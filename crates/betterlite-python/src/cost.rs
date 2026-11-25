mod billing;
mod breakdown;
pub(crate) mod convert;
pub(crate) mod engine;
pub(crate) mod error;
mod native;
pub(crate) mod settings;
use self::native::NativeCost;
use pyo3::prelude::*;
// The classes betterlite._cost reaches for: one pricing engine per settings text.
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeCost>()?;
    Ok(())
}
