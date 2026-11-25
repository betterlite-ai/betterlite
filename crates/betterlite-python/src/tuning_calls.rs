use pyo3::prelude::*;

mod convert;
mod error;
mod http;
mod session;

use self::error::NativeTuningError;
use self::session::NativeTuningSession;

// The classes fine_tuning.main reaches for: one session per call and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeTuningSession>()?;
    module.add(
        "NativeTuningError",
        module.py().get_type::<NativeTuningError>(),
    )?;
    Ok(())
}
