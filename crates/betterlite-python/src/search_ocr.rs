mod convert;
mod error;
mod native;
mod ocr_client;
mod pricing;
mod session;
mod tests;

use pyo3::prelude::*;

// The classes litellm.search, litellm.search.cost and litellm.ocr reach for: one session and its exception name.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<native::NativeSearchSession>()?;
    // every family raises the one NativeError class under its own name, so Python catches either
    module.setattr("NativeSearchError", module.getattr("NativeError")?)?;
    Ok(())
}
