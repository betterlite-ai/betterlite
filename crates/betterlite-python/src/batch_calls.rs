mod convert;
mod error;
mod payload;
mod session;
#[cfg(test)]
mod tests;
mod transport;

use self::error::NativeBatchError;
use self::session::NativeBatchSession;
use pyo3::prelude::*;

// The batch classes litellm.batches reaches for: one session and the exception every failure carries.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NativeBatchSession>()?;
    module.add(
        "NativeBatchError",
        module.py().get_type::<NativeBatchError>(),
    )?;
    Ok(())
}
