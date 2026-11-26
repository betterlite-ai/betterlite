mod adapter;
mod clients;
mod convert;
mod error;
mod native;
mod operations;
mod providers;
mod session;

use self::error::NativeVideoError;
use self::native::NativeVideoSession;
use pyo3::prelude::*;

// main.py's ten video entry points: one session, its five call methods in both flavours and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeVideoSession>()?;
    let error = module.py().get_type::<NativeVideoError>();
    module.add("NativeVideoError", error)
}
