mod convert;
mod edit;
mod error;
mod generate;
mod native;
mod session;
mod variation;

use self::error::NativeImageError;
use self::native::NativeImageSession;
use pyo3::prelude::*;

// main.py's six image entry points: one session, its six call methods and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeImageSession>()?;
    let error = module.py().get_type::<NativeImageError>();
    module.add("NativeImageError", error)
}
