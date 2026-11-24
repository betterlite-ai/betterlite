mod billing;
mod breakdown;
mod convert;
mod engine;
mod error;
mod native;
mod settings;
use self::native::NativeCost;
use pyo3::prelude::*;
// The classes betterlite._cost reaches for: one pricing engine per settings text.
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeCost>()?;
    Ok(())
}
