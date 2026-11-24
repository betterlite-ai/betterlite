use pyo3::prelude::*;

use self::functions::{cache_decode, cache_embedding, cache_envelope, cache_key};
use self::native::NativeCache;

mod convert;
mod core;
mod error;
mod functions;
mod native;
mod settings;
mod sink;

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeCache>()?;
    module.add_function(wrap_pyfunction!(cache_key, module)?)?;
    module.add_function(wrap_pyfunction!(cache_envelope, module)?)?;
    module.add_function(wrap_pyfunction!(cache_decode, module)?)?;
    module.add_function(wrap_pyfunction!(cache_embedding, module)?)?;
    Ok(())
}
